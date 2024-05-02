use core::time::Duration;

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use spinlock::SpinRaw;

use crate::{ats::{ATS_DRIVER, DRIVER_LOCK, GLOBAL_ATS_DRIVER, PROCESS_ID}, current, task::{AbsTaskInner, AxTask, TaskState}, AxTaskRef, CurrentTask};

/// A queue to store sleeping tasks.
///
/// # Examples
///
/// ```
/// use axtask::WaitQueue;
/// use core::sync::atomic::{AtomicU32, Ordering};
///
/// static VALUE: AtomicU32 = AtomicU32::new(0);
/// static WQ: WaitQueue = WaitQueue::new();
///
/// axtask::init_scheduler();
/// // spawn a new task that updates `VALUE` and notifies the main task
/// axtask::spawn(|| {
///     assert_eq!(VALUE.load(Ordering::Relaxed), 0);
///     VALUE.fetch_add(1, Ordering::Relaxed);
///     WQ.notify_one(true); // wake up the main task
/// });
///
/// WQ.wait(); // block until `notify()` is called
/// assert_eq!(VALUE.load(Ordering::Relaxed), 1);
/// ```
pub struct WaitQueue {
    queue: SpinRaw<VecDeque<AxTaskRef>>, // we already disabled IRQs when lock the `RUN_QUEUE`
}

impl WaitQueue {
    /// Creates an empty wait queue.
    pub const fn new() -> Self {
        Self {
            queue: SpinRaw::new(VecDeque::new()),
        }
    }

    /// Creates an empty wait queue with space for at least `capacity` elements.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            queue: SpinRaw::new(VecDeque::with_capacity(capacity)),
        }
    }

    fn cancel_events(&self, task: AxTaskRef) {
        // A task can be wake up only one events (timer or `notify()`), remove
        // the event from another queue.
        if task.in_wait_queue() {
            // wake up by timer (timeout).
            // `RUN_QUEUE` is not locked here, so disable IRQs.
            let _guard = kernel_guard::IrqSave::new();
            self.queue.lock().retain(|t| !Arc::ptr_eq(&task, t));
            task.set_in_wait_queue(false);
        }
        #[cfg(feature = "irq")]
        if task.in_timer_list() {
            // timeout was set but not triggered (wake up by `WaitQueue::notify()`)
            crate::timers::cancel_alarm(&task);
        }
    }

    /// Blocks the current task and put it into the wait queue, until other task
    /// notifies it.
    pub fn wait(&self) {
        // RUN_QUEUE.lock().block_current(|task| {
        //     task.set_in_wait_queue(true);
        //     self.queue.lock().push_back(task)
        // });
        let task = crate::current().unwrap();
        task.set_in_wait_queue(false); // 为了防止任务在返回调度器之前（上下文还未更新）就被唤醒，因此在任务返回调度器后再设置`in_wait_queue`标记。
        self.queue.lock().push_back(task.clone());
        // error!("before set in wait queue");
        task.clone().sync_block(|task| {
            task.set_in_wait_queue(true); // 若任务位于队列中，但`in_wait_queue`标记为`false`，则视为不在队列中。如果此时队列要弹出这个任务，则会忙等。因此任务入队和设置标记之间不能间隔太长。
            // error!("after set in wait queue");
        });
        self.cancel_events(task);
    }

    pub async fn wait_async(&self) {
        // RUN_QUEUE.lock().block_current(|task| {
        //     task.set_in_wait_queue(true);
        //     self.queue.lock().push_back(task)
        // });
        let task = crate::current().unwrap();
        task.set_in_wait_queue(false); // 为了防止任务在返回调度器之前（上下文还未更新）就被唤醒，因此在任务返回调度器后再设置`in_wait_queue`标记。
        self.queue.lock().push_back(task.clone());
        // error!("before set in wait queue");
        task.clone().async_block(|task| {
            task.set_in_wait_queue(true); // 若任务位于队列中，但`in_wait_queue`标记为`false`，则视为不在队列中。如果此时队列要弹出这个任务，则会忙等。因此任务入队和设置标记之间不能间隔太长。
            // error!("after set in wait queue");
        }).await;
        self.cancel_events(task);
    }

    // 将任务放入等待队列之后，在Executor上下文中执行额外动作
    pub fn wait_and<F>(&self, return_action: F)
    where F: FnOnce(AxTaskRef) + Send + Sync + 'static {
        // RUN_QUEUE.lock().block_current(|task| {
        //     task.set_in_wait_queue(true);
        //     self.queue.lock().push_back(task)
        // });
        let task = crate::current().unwrap();
        task.set_in_wait_queue(false);
        self.queue.lock().push_back(task.clone());
        // error!("before set in wait queue");
        task.clone().sync_block(|task| {
            task.set_in_wait_queue(true); 
            // error!("after set in wait queue");
            return_action(task);
        });
        self.cancel_events(task);
    }

    pub async fn wait_async_and<F>(&self, return_action: F)
    where F: FnOnce(AxTaskRef) + Send + Sync + 'static {
        // RUN_QUEUE.lock().block_current(|task| {
        //     task.set_in_wait_queue(true);
        //     self.queue.lock().push_back(task)
        // });
        let task = crate::current().unwrap();
        task.set_in_wait_queue(false);
        self.queue.lock().push_back(task.clone());
        // error!("before set in wait queue");
        task.clone().async_block(|task| {
            task.set_in_wait_queue(true);
            // error!("after set in wait queue");
            return_action(task);
        }).await;
        self.cancel_events(task);
    }

    /// Blocks the current task and put it into the wait queue, until the given
    /// `condition` becomes true.
    ///
    /// Note that even other tasks notify this task, it will not wake up until
    /// the condition becomes true.
    pub fn wait_until<F>(&self, condition: F)
    where
        F: Fn() -> bool,
    {
        // error!("wait_until");
        let task = crate::current().unwrap();
        loop {
            // let mut rq = RUN_QUEUE.lock();
            // if condition() {
            //     break;
            // }
            // rq.block_current(|task| {
            //     task.set_in_wait_queue(true);
            //     self.queue.lock().push_back(task);
            // });
            {
                let mut queue = self.queue.lock();
                if condition() {
                    break;
                }
                task.set_in_wait_queue(false);
                queue.push_back(task.clone());
            }
            // error!("before set in wait queue");
            task.clone().sync_block(|task| {
                task.set_in_wait_queue(true);
                // error!("after set in wait queue");
            });
        }
        self.cancel_events(task);
    }

    pub async fn wait_until_async<F>(&self, condition: F)
    where
        F: Fn() -> bool,
        {
            let task = crate::current().unwrap();
            loop {
                // let mut rq = RUN_QUEUE.lock();
                // if condition() {
                //     break;
                // }
                // rq.block_current(|task| {
                //     task.set_in_wait_queue(true);
                //     self.queue.lock().push_back(task);
                // });
                {
                    let mut queue = self.queue.lock();
                    if condition() {
                        break;
                    }
                    task.set_in_wait_queue(false);
                    queue.push_back(task.clone());
                }
                // error!("before set in wait queue");
                task.clone().async_block(|task| {
                    task.set_in_wait_queue(true);
                    // error!("after set in wait queue");
                }).await;
            }
            self.cancel_events(task);
        }

    /// Blocks the current task and put it into the wait queue, until other tasks
    /// notify it, or the given duration has elapsed.
    #[cfg(feature = "irq")]
    pub fn wait_timeout(&self, dur: core::time::Duration) -> bool {
        let curr = crate::current().unwrap();
        let deadline = axhal::time::current_time() + dur;
        debug!(
            "task wait_timeout: {} deadline={:?}",
            curr.id_name(),
            deadline
        );
        crate::timers::set_alarm_wakeup(deadline, curr.clone());

        // RUN_QUEUE.lock().block_current(|task| {
        //     task.set_in_wait_queue(true);
        //     self.queue.lock().push_back(task)
        // });
        curr.set_in_wait_queue(false);
        self.queue.lock().push_back(curr.clone());
        // error!("before set in wait queue");
        curr.clone().sync_block(|task| {
            task.set_in_wait_queue(true);
            // error!("after set in wait queue");
        });

        let timeout = curr.in_wait_queue(); // still in the wait queue, must have timed out
        self.cancel_events(curr);
        timeout
    }

    /// Blocks the current task and put it into the wait queue, until the given
    /// `condition` becomes true, or the given duration has elapsed.
    ///
    /// Note that even other tasks notify this task, it will not wake up until
    /// the above conditions are met.
    #[cfg(feature = "irq")]
    pub fn wait_timeout_until<F>(&self, dur: core::time::Duration, condition: F) -> bool
    where
        F: Fn() -> bool,
    {

        let curr = crate::current().unwrap();
        let deadline = axhal::time::current_time() + dur;
        debug!(
            "task wait_timeout: {}, deadline={:?}",
            curr.id_name(),
            deadline
        );
        crate::timers::set_alarm_wakeup(deadline, curr.clone());

        let mut timeout = true;
        while axhal::time::current_time() < deadline {
            // let mut rq = RUN_QUEUE.lock();
            {
                let mut queue = self.queue.lock(); 
                if condition() {
                    timeout = false;
                    break;
                }
                // rq.block_current(|task| {
                //     task.set_in_wait_queue(true);
                //     self.queue.lock().push_back(task);
                // });
                curr.set_in_wait_queue(false);
                queue.push_back(curr.clone());
            }

            // error!("before set in wait queue");
            curr.clone().sync_block(|task| {
                task.set_in_wait_queue(true);
                // error!("after set in wait queue");
            });
        }
        self.cancel_events(curr);
        timeout
    }

    /// Wakes up one task in the wait queue, usually the first one.
    ///
    /// If `resched` is true, the current task will be preempted when the
    /// preemption is enabled.
    pub fn notify_one(&self, resched: bool) -> bool {
        // let mut rq = RUN_QUEUE.lock();
        if !self.queue.lock().is_empty() {
            self.notify_one_locked(resched)
        } else {
            false
        }
    }

    /// Wakes all tasks in the wait queue.
    ///
    /// If `resched` is true, the current task will be preempted when the
    /// preemption is enabled.
    pub fn notify_all(&self, resched: bool) {
        loop {
            // let mut rq = RUN_QUEUE.lock();
            if let Some(task) = self.queue.lock().pop_front() {
                while !task.in_wait_queue() { }
                task.set_in_wait_queue(false);
                // rq.unblock_task(task, resched);
                task.set_state(TaskState::Ready);
                let priority = task.get_priority();
                let task_ref = task.into_task_ref();
                unsafe {
                    // let lock = DRIVER_LOCK.lock();
                    // let driver = ATS_DRIVER.current_ref_raw();
                    let driver = GLOBAL_ATS_DRIVER.lock();
                    driver.ps_push(task_ref, priority);
                }
            } else {
                break;
            }
            // drop(rq); // we must unlock `RUN_QUEUE` after unlocking `self.queue`.
        }
    }

    /// Wake up the given task in the wait queue.
    ///
    /// If `resched` is true, the current task will be preempted when the
    /// preemption is enabled.
    pub fn notify_task(&self, resched: bool, task: &AxTaskRef) -> bool {
        // let mut rq = RUN_QUEUE.lock();
        let mut wq = self.queue.lock();
        if let Some(index) = wq.iter().position(|t| Arc::ptr_eq(t, task)) {
            while !task.in_wait_queue() { }
            wq.remove(index);
            task.set_in_wait_queue(false);
            // rq.unblock_task(wq.remove(index).unwrap(), resched);
            task.set_state(TaskState::Ready);
            let task_ref = task.clone().into_task_ref();
            unsafe {
                // let lock = DRIVER_LOCK.lock();
                // let driver = ATS_DRIVER.current_ref_raw();
                let driver = GLOBAL_ATS_DRIVER.lock();
                driver.ps_push(task_ref, task.get_priority());
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn notify_one_locked(&self, resched: bool) -> bool {
        if let Some(task) = self.queue.lock().pop_front() {
            // error!("before wait enqueue");
            while !task.in_wait_queue() { }
            // error!("after wait enqueue");
            task.set_in_wait_queue(false);
            // rq.unblock_task(task, resched);
            task.set_state(TaskState::Ready);
            let priority = task.get_priority();
            let task_ref = task.into_task_ref();
            unsafe {
                // let lock = DRIVER_LOCK.lock();
                // let driver = ATS_DRIVER.current_ref_raw();
                let driver = GLOBAL_ATS_DRIVER.lock();
                driver.ps_push(task_ref, priority);
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn notify_all_locked(&self, resched: bool) {
        while let Some(task) = self.queue.lock().pop_front() {
            while !task.in_wait_queue() { }
            if !task.in_wait_queue() {
                continue;
            }
            task.set_in_wait_queue(false);
            // rq.unblock_task(task, resched);
            task.set_state(TaskState::Ready);
            let priority = task.get_priority();
            let task_ref = task.into_task_ref();
            unsafe {
                // let lock = DRIVER_LOCK.lock();
                // let driver = ATS_DRIVER.current_ref_raw();
                let driver = GLOBAL_ATS_DRIVER.lock();
                driver.ps_push(task_ref, priority);
            }
        }
    }
}
