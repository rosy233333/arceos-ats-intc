use crate::BaseScheduler;
use ats_intc::{AtsIntc, TaskRef};
use core::{marker::PhantomData, ops::Deref};
use alloc::sync::Arc;

/// A task wrapper for the [`ATScheduler`].
///
pub struct AtsTask<T> {
    inner: T,
}

impl<T> AtsTask<T> {
    /// Creates a new [`AtsTask`] from the inner task struct.
    pub const fn new(inner: T) -> Self {
        Self {
            inner
        }
    }

    /// Returns a reference to the inner task struct.
    pub const fn inner(&self) -> &T {
        &self.inner
    }
}

impl<T> Deref for AtsTask<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// A cooperative scheduler using the `ATSINTC` interrupt controller.
///
/// When a task is added to the scheduler, it's placed at the end of the ready
/// queue which is in the interrupt controller. When picking the next task to run, the head of the ready queue is
/// taken.
///
/// As it's a cooperative scheduler, it does nothing when the timer tick occurs.
///
pub struct ATScheduler<T> {
    inner: AtsIntc,
    phantomdata: PhantomData<T>
}

impl<T> ATScheduler<T> {
    /// Creates a new empty [`ATScheduler`].
    pub const fn new(base_addr: usize) -> Self {
        Self {
            inner: AtsIntc::new(base_addr),
            phantomdata: PhantomData,
        }
    }
    /// get the name of scheduler
    pub fn scheduler_name() -> &'static str {
        "ats"
    }
}

impl<T> BaseScheduler for ATScheduler<T> {
    type SchedItem = Arc<AtsTask<T>>;

    fn init(&mut self) {}

    fn add_task(&mut self, task: Self::SchedItem) {
        let raw_ptr = Arc::into_raw(task) as usize;
        let task_ref = unsafe { TaskRef::virt_task(raw_ptr) };
        self.inner.ps_push(task_ref, 0)
    }

    fn remove_task(&mut self, _task: &Self::SchedItem) -> Option<Self::SchedItem> {
        unimplemented!()
    }

    fn pick_next_task(&mut self) -> Option<Self::SchedItem> {
        self.inner.ps_fetch().map(|task_ref| {
            let task = unsafe { Arc::from_raw(task_ref.as_ptr() as *const usize as *const _) };
            task
        })
    }

    fn put_prev_task(&mut self, prev: Self::SchedItem, _preempt: bool) {
        self.add_task(prev);
    }

    fn task_tick(&mut self, _current: &Self::SchedItem) -> bool {
        false // no reschedule
    }

    fn set_priority(&mut self, _task: &Self::SchedItem, _prio: isize) -> bool {
        false
    }
}
