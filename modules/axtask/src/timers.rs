use core::task::Waker;

use alloc::sync::Arc;
use axhal::time::current_time;
use lazy_init::LazyInit;
use spinlock::SpinNoIrq;
use timer_list::{TimeValue, TimerEvent, TimerList};

use crate::{ats::ATS_DRIVER, task::AxTask, AxTaskRef};

// TODO: per-CPU
static TIMER_LIST: LazyInit<SpinNoIrq<TimerList<TaskWakeupEvent>>> = LazyInit::new();

struct TaskWakeupEvent(AxTaskRef);

impl TimerEvent for TaskWakeupEvent {
    fn callback(self, _now: TimeValue) {
        // let mut rq = RUN_QUEUE.lock();
        self.0.set_in_timer_list(false);
        let priority = self.0.get_priority();
        let task_ref = self.0.into_task_ref();
        {
            let driver_lock = ATS_DRIVER.lock();
            driver_lock.ps_push(task_ref, priority);
        }
        // rq.unblock_task(self.0, true);
    }
}

pub fn set_alarm_wakeup(deadline: TimeValue, task: AxTaskRef) {
    let mut timers = TIMER_LIST.lock();
    task.set_in_timer_list(true);
    timers.set(deadline, TaskWakeupEvent(task));
}

pub fn cancel_alarm(task: &AxTaskRef) {
    let mut timers = TIMER_LIST.lock();
    task.set_in_timer_list(false);
    timers.cancel(|t| Arc::ptr_eq(&t.0, task));
}

pub fn check_events() {
    loop {
        let now = current_time();
        let event = TIMER_LIST.lock().expire_one(now);
        if let Some((_deadline, event)) = event {
            event.callback(now);
        } else {
            break;
        }
    }
}

pub fn init() {
    TIMER_LIST.init_by(SpinNoIrq::new(TimerList::new()));
}
