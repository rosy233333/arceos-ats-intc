//! Task APIs for multi-task configuration.

use alloc::{string::String, sync::Arc};

pub(crate) use crate::run_queue::{AxRunQueue, RUN_QUEUE};

#[doc(cfg(feature = "multitask"))]
pub use crate::task::{CurrentTask, TaskId, TaskInner};
#[doc(cfg(feature = "multitask"))]
pub use crate::wait_queue::WaitQueue;

/// The reference type of a task.
pub type AxTaskRef = Arc<AxTask>;

cfg_if::cfg_if! {
    if #[cfg(feature = "sched_rr")] {
        const MAX_TIME_SLICE: usize = 5;
        pub(crate) type AxTask = scheduler::RRTask<TaskInner, MAX_TIME_SLICE>;
        pub(crate) type Scheduler = scheduler::RRScheduler<TaskInner, MAX_TIME_SLICE>;
    } else if #[cfg(feature = "sched_cfs")] {
        pub(crate) type AxTask = scheduler::CFSTask<TaskInner>;
        pub(crate) type Scheduler = scheduler::CFScheduler<TaskInner>;
    } else if #[cfg(feature = "sched_atsintc")] {
        pub(crate) type AxTask = scheduler::AtsTask<TaskInner>;
        pub(crate) type Scheduler = scheduler::ATScheduler<TaskInner>;
    } else {
        // If no scheduler features are set, use FIFO as the default.
        pub(crate) type AxTask = scheduler::FifoTask<TaskInner>;
        pub(crate) type Scheduler = scheduler::FifoScheduler<TaskInner>;
    }
}

#[cfg(feature = "preempt")]
struct KernelGuardIfImpl;

#[cfg(feature = "preempt")]
#[crate_interface::impl_interface]
impl kernel_guard::KernelGuardIf for KernelGuardIfImpl {
    fn disable_preempt() {
        if let Some(curr) = current_may_uninit() {
            curr.disable_preempt();
        }
    }

    fn enable_preempt() {
        if let Some(curr) = current_may_uninit() {
            curr.enable_preempt(true);
        }
    }
}

/// Gets the current task, or returns [`None`] if the current task is not
/// initialized.
pub fn current_may_uninit() -> Option<CurrentTask> {
    CurrentTask::try_get()
}

/// Gets the current task.
///
/// # Panics
///
/// Panics if the current task is not initialized.
pub fn current() -> CurrentTask {
    CurrentTask::get()
}

/// Initializes the task scheduler (for the primary CPU).
pub fn init_scheduler() {
    info!("Initialize scheduling...");

    crate::run_queue::init();
    #[cfg(feature = "irq")]
    crate::timers::init();

    info!("  use {} scheduler.", Scheduler::scheduler_name());
}

/// Initializes the task scheduler for secondary CPUs.
pub fn init_scheduler_secondary() {
    crate::run_queue::init_secondary();
}

/// Handles periodic timer ticks for the task manager.
///
/// For example, advance scheduler states, checks timed events, etc.
#[cfg(feature = "irq")]
#[doc(cfg(feature = "irq"))]
pub fn on_timer_tick() {
    crate::timers::check_events();
    RUN_QUEUE.lock().scheduler_timer_tick();
}

/// Spawns a new task with the given parameters.
///
/// Returns the task reference.
pub fn spawn_raw<F>(f: F, name: String, stack_size: usize) -> AxTaskRef
where
    F: FnOnce() + Send + 'static,
{
    let task = TaskInner::new(f, name, stack_size);
    RUN_QUEUE.lock().add_task(task.clone());
    task
}

/// Spawns a new task with the default parameters.
///
/// The default task name is an empty string. The default task stack size is
/// [`axconfig::TASK_STACK_SIZE`].
///
/// Returns the task reference.
pub fn spawn<F>(f: F) -> AxTaskRef
where
    F: FnOnce() + Send + 'static,
{
    spawn_raw(f, "".into(), axconfig::TASK_STACK_SIZE)
}

/// Set the priority for current task.
///
/// The range of the priority is dependent on the underlying scheduler. For
/// example, in the [CFS] scheduler, the priority is the nice value, ranging from
/// -20 to 19.
///
/// Returns `true` if the priority is set successfully.
///
/// [CFS]: https://en.wikipedia.org/wiki/Completely_Fair_Scheduler
pub fn set_priority(prio: isize) -> bool {
    RUN_QUEUE.lock().set_current_priority(prio)
}

/// Current task gives up the CPU time voluntarily, and switches to another
/// ready task.
pub fn yield_now() {
    RUN_QUEUE.lock().yield_current();
}

/// Current task is going to sleep for the given duration.
///
/// If the feature `irq` is not enabled, it uses busy-wait instead.
pub fn sleep(dur: core::time::Duration) {
    sleep_until(axhal::time::current_time() + dur);
}

/// Current task is going to sleep, it will be woken up at the given deadline.
///
/// If the feature `irq` is not enabled, it uses busy-wait instead.
pub fn sleep_until(deadline: axhal::time::TimeValue) {
    #[cfg(feature = "irq")]
    RUN_QUEUE.lock().sleep_until(deadline);
    #[cfg(not(feature = "irq"))]
    axhal::time::busy_wait_until(deadline);
}

/// Exits the current task.
pub fn exit(exit_code: i32) -> ! {
    RUN_QUEUE.lock().exit_current(exit_code)
}

#[cfg(feature = "sched_atsintc")]
pub use block::*;
#[cfg(feature = "sched_atsintc")]
mod block {
    use super::AxTaskRef;
    use crate::RUN_QUEUE;
    use spinlock::SpinNoIrq;
    use alloc::collections::VecDeque;
    pub static NET_WAIT: SpinNoIrq<VecDeque<AxTaskRef>> = SpinNoIrq::new(VecDeque::new());
    pub fn block() {
        RUN_QUEUE.lock().block_current(|task| {
            // change the task
            task.set_in_wait_queue(true);
            debug!("{} is waiting for net request", task.id_name());
            NET_WAIT.lock().push_back(task);
        });
    }
}

cfg_if::cfg_if! {
    if #[cfg(feature = "async")] {
        use core::future::poll_fn;
        use core::task::Poll;
        use crate::run_queue::IDLE_TASK;
        use ats_intc::{Task, TaskType, AtsIntc};
        use alloc::boxed::Box;
        use crate::task::TaskState;
        static ATSINTC: AtsIntc = AtsIntc::new(0xffff_ffc0_0f00_0000);

        fn pick_next() -> Option<AxTaskRef> {
            RUN_QUEUE.lock().pick_next()
        }

        fn axtask_run(axtask: Arc<AxTask>) {
            axtask.set_state(TaskState::Running);
            let ctx_ptr = unsafe { &*axtask.ctx_mut_ptr() };
            unsafe { CurrentTask::set_current(CurrentTask::get(), axtask) };
            let fut = poll_fn(|_cx| {
                unsafe { (*IDLE_TASK.current_ref_mut_raw().ctx_mut_ptr()).switch_to(ctx_ptr); }
                Poll::<i32>::Pending
            });
            let task_ref = Task::new(Box::pin(fut), 0, TaskType::Process, &ATSINTC);
            let _ = task_ref.poll();
        }
    }
}

/// The idle task routine.
///
/// It runs an infinite loop that keeps calling [`yield_now()`].
pub fn run_idle() -> ! {
    loop {
        #[cfg(not(feature = "async"))]
        yield_now();
        #[cfg(feature = "async")]
        if let Some(task) = pick_next() {
            axtask_run(task.clone());
        }
        debug!("idle task: waiting for IRQs...");
        #[cfg(feature = "irq")]
        axhal::arch::wait_for_irqs();
    }
}
