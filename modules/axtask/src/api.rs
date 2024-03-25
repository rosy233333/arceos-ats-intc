//! Task APIs for multi-task configuration.

use alloc::{string::String, sync::Arc};
use ats_intc::AtsIntc;

use crate::ats::{Ats, ATS_DRIVER, ATS_EXECUTOR, CURRENT_TASK};
// pub(crate) use crate::run_queue::{AxRunQueue, RUN_QUEUE};

use crate::task::{AbsTaskInner, AsyncTaskInner, AxTask};
#[doc(cfg(feature = "multitask"))]
pub use crate::task::{CurrentTask, TaskId, TaskInner};
#[doc(cfg(feature = "multitask"))]
pub use crate::wait_queue::WaitQueue;

use crate::ats::PROCESS_ID;

/// The reference type of a task.
pub type AxTaskRef = Arc<AxTask>;

use core::cell::RefCell;
use core::future::Future;

// cfg_if::cfg_if! {
//     if #[cfg(feature = "sched_rr")] {
//         const MAX_TIME_SLICE: usize = 5;
//         pub(crate) type AxTask = scheduler::RRTask<TaskInner, MAX_TIME_SLICE>;
//         pub(crate) type Scheduler = scheduler::RRScheduler<TaskInner, MAX_TIME_SLICE>;
//     } else if #[cfg(feature = "sched_cfs")] {
//         pub(crate) type AxTask = scheduler::CFSTask<TaskInner>;
//         pub(crate) type Scheduler = scheduler::CFScheduler<TaskInner>;
//     } else {
//         // If no scheduler features are set, use FIFO as the default.
//         pub(crate) type AxTask = scheduler::FifoTask<TaskInner>;
//         pub(crate) type Scheduler = scheduler::FifoScheduler<TaskInner>;
//     }
// }

// #[cfg(feature = "preempt")]
// struct KernelGuardIfImpl;

// #[cfg(feature = "preempt")]
// #[crate_interface::impl_interface]
// impl kernel_guard::KernelGuardIf for KernelGuardIfImpl {
//     fn disable_preempt() {
//         if let Some(curr) = current_may_uninit() {
//             curr.disable_preempt();
//         }
//     }

//     fn enable_preempt() {
//         if let Some(curr) = current_may_uninit() {
//             curr.enable_preempt(true);
//         }
//     }
// }

// /// Gets the current task, or returns [`None`] if the current task is not
// /// initialized.
// pub fn current_may_uninit() -> Option<CurrentTask> {
//     CurrentTask::try_get()
// }

/// Init the scheduler/executor.
pub fn init() {
    ATS_DRIVER.init_by(AtsIntc::new(0xffff_ffc0_0f00_0000));
    ATS_EXECUTOR.init_by(Ats::new(PROCESS_ID));
    CURRENT_TASK.init_by(CurrentTask::new());
}

/// Gets the current task.
pub fn current() -> Option<AxTaskRef> {
    CURRENT_TASK.clone()
}

/// Gets the current task id.
pub fn current_id() -> Option<u64> {
    current().map(|t|{ t.id().as_u64() })
}

// /// Initializes the task scheduler (for the primary CPU).
// pub fn init_scheduler() {
//     info!("Initialize scheduling...");

//     crate::run_queue::init();
//     #[cfg(feature = "irq")]
//     crate::timers::init();

//     info!("  use {} scheduler.", Scheduler::scheduler_name());
// }

// /// Initializes the task scheduler for secondary CPUs.
// pub fn init_scheduler_secondary() {
//     crate::run_queue::init_secondary();
// }

/// Run the task executor
pub fn run_executor() -> ! {
    ATS_EXECUTOR.run()
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
    let priority = task.inner.get_priority();
    let task_ref = Arc::into_raw(task.clone()) as *const () as usize;
    ATS_DRIVER.ps_push(task_ref, priority);
    task
}

pub fn spawn_raw_init<F>(f: F, name: String, stack_size: usize) -> AxTaskRef
where
    F: FnOnce() + Send + 'static,
{
    let task = TaskInner::new_init(f, name, stack_size);
    let priority = task.inner.get_priority();
    let task_ref = Arc::into_raw(task.clone()) as *const () as usize;
    ATS_DRIVER.ps_push(task_ref, priority);
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

pub fn spawn_init<F>(f: F) -> AxTaskRef
where
    F: FnOnce() + Send + 'static,
{
    spawn_raw_init(f, "main".into(), axconfig::TASK_STACK_SIZE)
}

/// Spawns a new async task with the given parameters.
///
/// Returns the task reference.
pub fn spawn_raw_async<F>(f: F, name: String) -> AxTaskRef
where
    F: Future<Output = i32> + Send + Sync + 'static,
{
    let task = AsyncTaskInner::new(f, name);
    let priority = task.inner.get_priority();
    let task_ref = Arc::into_raw(task.clone()) as *const () as usize;
    ATS_DRIVER.ps_push(task_ref, priority);
    task
}

/// Spawns a new task with the default parameters.
///
/// The default task name is an empty string. The default task stack size is
/// [`axconfig::TASK_STACK_SIZE`].
///
/// Returns the task reference.
pub fn spawn_async<F>(f: F) -> AxTaskRef
where
    F: Future<Output = i32> + Send + Sync + 'static,
{
    spawn_raw_async(f, "".into())
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
    if prio >= 0 {
        let current = current();
        current.unwrap().set_priority(prio as usize);
        true
    }
    else {
        false
    }
}

/// Current task gives up the CPU time voluntarily, and switches to another
/// ready task.
pub fn yield_now() {
    let current_task = current().unwrap();
    current_task.sync_yield();
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
    // TODO
    // #[cfg(feature = "irq")]
    // RUN_QUEUE.lock().sleep_until(deadline);
    #[cfg(not(feature = "irq"))]
    axhal::time::busy_wait_until(deadline);
}

/// Exits the current task.
pub fn exit(exit_code: i32) -> ! {
    let current_task = current().unwrap();
    current_task.sync_exit(exit_code);
}

// /// The idle task routine.
// ///
// /// It runs an infinite loop that keeps calling [`yield_now()`].
// pub fn run_idle() -> ! {
//     loop {
//         yield_now();
//         debug!("idle task: waiting for IRQs...");
//         #[cfg(feature = "irq")]
//         axhal::arch::wait_for_irqs();
//     }
// }
