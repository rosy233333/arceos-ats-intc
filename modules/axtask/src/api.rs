//! Task APIs for multi-task configuration.

use alloc::{string::String, sync::Arc};
use ats_intc::AtsIntc;
use axconfig::SMP;
use axhal::cpu::this_cpu_id;
use alloc::vec;
use alloc::vec::Vec;
use spinlock::SpinNoIrq;

use crate::ats::{Ats, ATS_DRIVER, ATS_EXECUTORS, CURRENT_TASKS, DRIVER_LOCK, GLOBAL_ATS_DRIVER};
// pub(crate) use crate::run_queue::{AxRunQueue, RUN_QUEUE};

use crate::task::{AbsTaskInner, AsyncInner, AxTask};
#[doc(cfg(feature = "multitask"))]
pub use crate::task::{CurrentTask, TaskId, SyncInner};
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
    // ATS_EXECUTORS.init_by(vec![Ats::new(PROCESS_ID); SMP]);
    // for i in 0 .. SMP {
    //     ATS_EXECUTORS[i].cpu_id.init_by(i);
    // }
    // CURRENT_TASKS.init_by(SpinNoIrq::new(vec![CurrentTask::new(); SMP]));
    // GLOBAL_ATS_DRIVER.init_by(SpinNoIrq::new(AtsIntc::new(0xffff_ffc0_0f00_0000)));
    {
        let driver = GLOBAL_ATS_DRIVER.lock();
        assert!(!driver.is_init());
        driver.init_by(AtsIntc::new(0xffff_ffc0_0f00_0000));
        // driver.init_by(AtsIntc::new());
    }
    unsafe {
        // ATS_DRIVER.current_ref_raw().init_by(AtsIntc::new(0xffff_ffc0_0f00_0000));
        ATS_EXECUTORS.current_ref_raw().init_by(Ats::new(PROCESS_ID));
        CURRENT_TASKS.current_ref_raw().init_by(CurrentTask::new());
    }
    
    #[cfg(feature = "irq")]
    crate::timers::init();
}

pub fn init_secondary() {
    {
        let driver = GLOBAL_ATS_DRIVER.lock();
        assert!(driver.is_init());
    }
    unsafe {
        // ATS_DRIVER.current_ref_raw().init_by(AtsIntc::new(0xffff_ffc0_0f00_0000));
        ATS_EXECUTORS.current_ref_raw().init_by(Ats::new(PROCESS_ID));
        CURRENT_TASKS.current_ref_raw().init_by(CurrentTask::new());
    }
}

/// Gets the current task.
pub fn current() -> Option<AxTaskRef> {
    let cpu_id = this_cpu_id();
    // {
    //     let ct_lock = CURRENT_TASKS.lock();
    //     ct_lock[cpu_id].get_clone()
    // }
    unsafe {
        CURRENT_TASKS.current_ref_raw().get_clone()
    }
}

/// Gets the current task id.
pub fn current_id() -> Option<u64> {
    Some(current().map(|t|{ t.id().as_u64() }).unwrap_or(0))
}

pub fn register_irq_handler<F>(irq_num: usize, handler: F) -> bool
where
    F: Fn() + Send + 'static,
{
    let task = SyncInner::new(handler, "".into(), axconfig::TASK_STACK_SIZE);
    task.set_priority(0);
    unsafe {
        // let lock = DRIVER_LOCK.lock();
        // let driver = ATS_DRIVER.current_ref_raw();
        let driver = GLOBAL_ATS_DRIVER.lock();
        driver.intr_push(irq_num, task.into_task_ref());
    }
    
    true
}

pub fn register_async_irq_handler<F>(irq_num: usize, handler: F) -> bool
where
    F: Future<Output = i32> + Send + Sync + 'static,
{
    let task = AsyncInner::new(handler, "".into());
    task.set_priority(0);
    unsafe {
        // let lock = DRIVER_LOCK.lock();
        // let driver = ATS_DRIVER.current_ref_raw();
        let driver = GLOBAL_ATS_DRIVER.lock();
        driver.intr_push(irq_num, task.into_task_ref());
    }
    true
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
    unsafe {
        ATS_EXECUTORS.current_ref_raw().run()
    }
}

/// Handles periodic timer ticks for the task manager.
///
/// For example, advance scheduler states, checks timed events, etc.
#[cfg(feature = "irq")]
#[doc(cfg(feature = "irq"))]
pub fn on_timer_tick() {
    crate::timers::check_events();
    // RUN_QUEUE.lock().scheduler_timer_tick();
}

/// Spawns a new task with the given parameters.
///
/// Returns the task reference.
pub fn spawn_raw<F>(f: F, name: String, stack_size: usize) -> AxTaskRef
where
    F: FnOnce() + Send + 'static,
{
    let task = SyncInner::new(f, name, stack_size);
    let priority = task.get_priority();
    let task_ref = task.clone().into_task_ref();
    // let leak = Arc::into_raw(task.clone()); // tempoary solution
    unsafe {
        // let lock = DRIVER_LOCK.lock();
        // let driver = ATS_DRIVER.current_ref_raw();
        let driver = GLOBAL_ATS_DRIVER.lock();
        driver.ps_push(task_ref, priority);
    }
    task
}

pub fn spawn_raw_init<F>(f: F, name: String, stack_size: usize) -> AxTaskRef
where
    F: FnOnce() + Send + 'static,
{
    let task = SyncInner::new_init(f, name, stack_size);
    let priority = task.get_priority();
    let task_ref = task.clone().into_task_ref();
    // let leak = Arc::into_raw(task.clone()); // tempoary solution
    unsafe {
        // let lock = DRIVER_LOCK.lock();
        // let driver = ATS_DRIVER.current_ref_raw();
        let driver = GLOBAL_ATS_DRIVER.lock();
        driver.ps_push(task_ref, priority);
    }
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
    let task = AsyncInner::new(f, name);
    let priority = task.get_priority();
    let task_ref = task.clone().into_task_ref();
    // let leak = Arc::into_raw(task.clone()); // tempoary solution
    unsafe {
        // let lock = DRIVER_LOCK.lock();
        // let driver = ATS_DRIVER.current_ref_raw();
        let driver = GLOBAL_ATS_DRIVER.lock();
        driver.ps_push(task_ref, priority);
    }
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
    #[cfg(feature = "irq")] {
        // RUN_QUEUE.lock().sleep_until(deadline);
        let current_task = current().unwrap();
        current_task.sync_sleep_until(deadline);
    }
    #[cfg(not(feature = "irq"))]
    axhal::time::busy_wait_until(deadline);
}

pub async fn async_sleep(dur: core::time::Duration) {
    async_sleep_until(axhal::time::current_time() + dur).await;
}

pub async fn async_sleep_until(deadline: axhal::time::TimeValue) {
    #[cfg(feature = "irq")] {
        // RUN_QUEUE.lock().sleep_until(deadline);
        let current_task = current().unwrap();
        current_task.async_sleep_until(deadline).await;
    }
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
