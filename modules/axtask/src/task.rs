use alloc::task::Wake;
use alloc::{boxed::Box, string::String, sync::Arc};
use core::borrow::BorrowMut;
use core::cell::RefCell;
use core::future::Future;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use core::task::{Context, Poll};
use core::{alloc::Layout, cell::UnsafeCell, fmt, ptr::NonNull};

use core::arch::asm;
use ats_intc::{AtsIntc, TaskRef};

#[cfg(feature = "preempt")]
use core::sync::atomic::AtomicUsize;

#[cfg(feature = "tls")]
use axhal::tls::TlsArea;

use axhal::arch::TaskContext;
use memory_addr::{align_up_4k, VirtAddr, PAGE_SIZE_4K};

use crate::ats::{DRIVER_LOCK, GLOBAL_ATS_DRIVER, PROCESS_ID, WAIT_FOR_EXIT};
use crate::ats::EXITED_TASKS;
use crate::{AxTaskRef, WaitQueue};
use crate::ats::ATS_DRIVER;

pub trait AbsTaskInner: Downcast + TaskInfo + Send + Sync { }
impl AbsTaskInner for SyncInner { }
impl AbsTaskInner for AsyncInner { }

pub struct AxTask {
    pub task_inner: Box<dyn AbsTaskInner>,
    pub general_inner: GeneralInner,
}

impl Wake for AxTask {
    fn wake(self: Arc<Self>) {
        self.task_inner.set_state(TaskState::Ready);
        let priority = self.task_inner.get_priority();
        let task_ref = self.into_task_ref();
        unsafe {
            // let lock = DRIVER_LOCK.lock();
            // let driver = ATS_DRIVER.current_ref_raw();
            let driver = GLOBAL_ATS_DRIVER.lock();
            driver.ps_push(task_ref, priority);
        }
    }
}

impl AxTask {
    pub(crate) fn new_sync(inner: SyncInner) -> Self {
        Self {
            task_inner: Box::new(inner),
            general_inner: GeneralInner::new(),
        }
    }

    pub(crate) fn new_async(inner: AsyncInner) -> Self {
        Self {
            task_inner: Box::new(inner),
            general_inner: GeneralInner::new(),
        }
    }

    pub(crate) fn register_return_action<F>(&self, action: F)
    where F: FnOnce(AxTaskRef) + Send + Sync + 'static {
        unsafe {
            *self.general_inner.return_action.get() = Some(Box::into_raw(Box::new(action)));
        }
    }

    pub(crate) fn clear_return_action(&self) {
        unsafe {
            *self.general_inner.return_action.get() = None;
        }
    }

    pub(crate) fn poll(&self, cx: &mut Context<'_>) -> Poll<i32> {
        // self.inner.poll(cx)
        self.set_state(TaskState::Running);
        self.clear_return_action();
        if self.is_async() {
            let inner = self.task_inner.to_async_task_inner().unwrap();
            let res = unsafe { inner.fut.get().as_mut().unwrap().as_mut().poll(cx) };
            match &res {
                Poll::Pending => {
                    self.set_state(TaskState::Blocked);
                },
                Poll::Ready(value) => {
                    inner.exit_code.store(*value, Ordering::Release);
                    self.set_state(TaskState::Exited);
                    self.register_return_action(|task| {
                        let inner = task.task_inner.to_async_task_inner().unwrap();
                        inner.wait_for_exit.notify_all(false);
                    });
                }
            }
            res
        }
        else {
            info!("into poll");
            let inner = self.task_inner.to_task_inner().unwrap();
            unsafe {
                let old_ctx = inner.ret_ctx_mut_ptr().as_mut().unwrap();
                let new_ctx = inner.ctx_mut_ptr().as_ref().unwrap();
                old_ctx.switch_to_receive_exit_value(new_ctx)
            }
        }
    }

    pub fn id(&self) -> TaskId {
        self.task_inner.id()
    }

    pub(crate) fn name(&self) -> &str {
        self.task_inner.name()
    }

    pub fn id_name(&self) -> String {
        self.task_inner.id_name()
    }

    pub(crate) fn state(&self) -> TaskState {
        self.task_inner.state()
    }

    pub(crate) fn set_state(&self, state: TaskState) {
        self.task_inner.set_state(state)
    }

    pub(crate) fn is_running(&self) -> bool {
        self.task_inner.is_running()
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.task_inner.is_ready()
    }

    pub(crate) fn is_blocked(&self) -> bool {
        self.task_inner.is_blocked()
    }

    pub(crate) fn is_exited(&self) -> bool {
        self.task_inner.is_exited()
    }

    pub(crate) fn get_priority(&self) -> usize {
        self.task_inner.get_priority()
    }

    pub(crate) fn set_priority(&self, priority: usize) {
        self.task_inner.set_priority(priority)
    }

    pub(crate) fn is_init(&self) -> bool {
        self.task_inner.is_init()
    }

    pub(crate) fn is_idle(&self) -> bool {
        self.task_inner.is_idle()
    }

    pub(crate) fn in_wait_queue(&self) -> bool {
        self.task_inner.in_wait_queue()
    }

    pub(crate) fn set_in_wait_queue(&self, in_wait_queue: bool) {
        self.task_inner.set_in_wait_queue(in_wait_queue)
    }

    #[cfg(feature = "irq")]
    pub(crate) fn in_timer_list(&self) -> bool {
        self.task_inner.in_timer_list()
    }

    #[cfg(feature = "irq")]
    pub(crate) fn set_in_timer_list(&self, in_timer_list: bool) {
        self.task_inner.set_in_timer_list(in_timer_list)
    }

    pub(crate) fn is_async(&self) -> bool {
        self.task_inner.is_async()
    }

    pub fn sync_yield(self: Arc<Self>) {
        assert!(!self.is_async());
        assert!(self.is_running());

        self.set_state(TaskState::Ready);
        self.register_return_action(|task| {
            let priority = task.get_priority();
            unsafe {
                // let driver = ATS_DRIVER.current_ref_raw();
                let driver = GLOBAL_ATS_DRIVER.lock();
                driver.ps_push(task.into_task_ref(), priority);
            }
        });

        let inner = self.task_inner.to_task_inner().unwrap();
        unsafe {
            // let inner = &*(self.inner.as_ref() as *const dyn AbsTaskInner as *const () as *const TaskInner);
            let old_ctx = inner.ctx_mut_ptr().as_mut().unwrap();
            let new_ctx = inner.ret_ctx_mut_ptr().as_ref().unwrap();
            old_ctx.switch_to_return_pending(new_ctx);
        }
    }

    pub fn join(&self) -> Option<i32> {
        if self.task_inner.is_async() {
            let inner = self.task_inner.to_async_task_inner().unwrap();
            inner.join()
        }
        else {
            let inner = self.task_inner.to_task_inner().unwrap();
            inner.join()
        }
    }

    /// block the current task, then perform the `action` (usually to put this task into the block queue). 
    pub(crate) fn sync_block<F>(self: Arc<Self>, action: F)
    where F: FnOnce(AxTaskRef) + Send + Sync + 'static {
        assert!(!self.is_async());
        assert!(self.is_running());

        self.set_state(TaskState::Blocked);
        self.register_return_action(action);

        let inner = self.task_inner.to_task_inner().unwrap();
        unsafe {
            // let inner = &*(self.inner.as_ref() as *const dyn AbsTaskInner as *const () as *const TaskInner);
            let old_ctx = inner.ctx_mut_ptr().as_mut().unwrap();
            let new_ctx = inner.ret_ctx_mut_ptr().as_ref().unwrap();
            old_ctx.switch_to_return_pending(new_ctx);
        }
    }

    pub(crate) async fn async_block<F>(self: Arc<Self>, action: F)
    where F: FnOnce(AxTaskRef) + Send + Sync + 'static {
        assert!(self.is_async());
        assert!(self.is_running());

        struct AsyncBlock(UnsafeCell<usize>);
        impl AsyncBlock {
            fn new() -> Self {
                Self{0: UnsafeCell::new(0)}
            }
        }
        // unsafe impl Send for AsyncBlock { }
        unsafe impl Sync for AsyncBlock { }
        impl Future for AsyncBlock {
            type Output = ();
        
            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                if unsafe { *self.0.get() } == 0 {
                    unsafe {
                        *self.0.get() = 1;
                    }
                    Poll::Pending
                }
                else if unsafe { *self.0.get() } == 1 {
                    unsafe {
                        *self.0.get() = 2;
                    }
                    Poll::Ready(())
                }
                else {
                    panic!("async_block: invalid state");
                    Poll::Pending
                }
            }
        }

        self.register_return_action(action);
        let block = Box::pin(AsyncBlock::new());
        block.await;
    }

    pub fn sync_sleep_until(self: Arc<Self>, deadline: axhal::time::TimeValue) {
        assert!(self.is_running());
        assert!(!self.is_async());

        let now = axhal::time::current_time();
        if now < deadline {
            self.register_return_action(move |task| {
                let now = axhal::time::current_time();
                if now < deadline {
                    crate::timers::set_alarm_wakeup(deadline, task);
                }
                else {
                    // directly push to ready queue
                    let priority = task.get_priority();
                    unsafe {
                        // let driver = ATS_DRIVER.current_ref_raw();
                        let driver = GLOBAL_ATS_DRIVER.lock();
                        driver.ps_push(task.into_task_ref(), priority);
                    }
                }
            });
            self.set_state(TaskState::Blocked);
            let inner = self.task_inner.to_task_inner().unwrap();
            unsafe {
                // let inner = &*(self.inner.as_ref() as *const dyn AbsTaskInner as *const () as *const TaskInner);
                let old_ctx = inner.ctx_mut_ptr().as_mut().unwrap();
                let new_ctx = inner.ret_ctx_mut_ptr().as_ref().unwrap();
                old_ctx.switch_to_return_pending(new_ctx);
            }
        }
    }

    pub async fn async_sleep_until(self: Arc<Self>, deadline: axhal::time::TimeValue) {
        struct AsyncSleepUntil {
            task: Arc<AxTask>,
            deadline: axhal::time::TimeValue,
        }

        impl Future for AsyncSleepUntil {
            type Output = ();
        
            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                assert!(self.task.is_running());
                let now = axhal::time::current_time();
                if now < self.deadline {
                    let deadline = self.deadline;
                    self.task.register_return_action(move |task| {
                        let now = axhal::time::current_time();
                        if now < deadline {
                            crate::timers::set_alarm_wakeup(deadline, task);
                        }
                        else {
                            // directly push to ready queue
                            let priority = task.get_priority();
                            unsafe {
                                // let driver = ATS_DRIVER.current_ref_raw();
                                let driver = GLOBAL_ATS_DRIVER.lock();
                                driver.ps_push(task.into_task_ref(), priority);
                            }
                        }
                    });
                    Poll::Pending
                }
                else {
                    Poll::Ready(())
                }
            }
        }

        AsyncSleepUntil { task: self, deadline }.await
    }

    pub fn sync_exit(self: Arc<Self>, exit_code: i32) -> ! {
        assert!(!self.is_async());
        assert!(self.is_running());
        assert!(!self.is_idle());
        info!("into exit");
        // let inner = unsafe { &*(self.inner.as_ref() as *const dyn AbsTaskInner as *const () as *const TaskInner) };
        let inner = self.task_inner.to_task_inner().unwrap();
        inner.exit_code.store(exit_code, Ordering::Release);
        self.set_state(TaskState::Exited);
        self.register_return_action(|task| {
            // if task.is_init() {
            //     EXITED_TASKS.lock().clear();
            //     axhal::misc::terminate();
            // }
            let inner = task.task_inner.to_task_inner().unwrap();
            inner.wait_for_exit.notify_all_locked(false);
            EXITED_TASKS.lock().push_back(task);
            WAIT_FOR_EXIT.notify_one_locked(false);
        });
        
        unsafe {
            let old_ctx = inner.ctx_mut_ptr().as_mut().unwrap();
            let new_ctx = inner.ret_ctx_mut_ptr().as_ref().unwrap();
            old_ctx.switch_to_return_ready(new_ctx, exit_code);
        }

        unreachable!();
    }

    /// SAFETY: `task_ref` must be generated from `Self::into_task_ref`
    pub(crate) unsafe fn from_task_ref(task_ref: TaskRef) -> Arc<Self> {
        Arc::from_raw(task_ref.as_ptr() as *const () as *const Self)
    } 

    pub(crate) fn into_task_ref(self: Arc<Self>) -> TaskRef {
        unsafe { TaskRef::virt_task(Arc::into_raw(self) as usize) }
    }
}

pub struct GeneralInner {
    pub return_action: UnsafeCell<Option<*mut (dyn FnOnce(AxTaskRef) + Send + Sync)>>, // the action that scheduler should do when task return (yield / block / exit)
}

unsafe impl Send for GeneralInner {}
unsafe impl Sync for GeneralInner {}

impl GeneralInner {
    pub fn new() -> Self {
        Self {
            return_action: UnsafeCell::new(None),
        }
    }
}

/// A unique identifier for a thread.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TaskId(u64);

/// The possible states of a task.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum TaskState {
    Running = 1,
    Ready = 2,
    Blocked = 3,
    Exited = 4,
}

/// The inner task structure.
pub struct SyncInner {
    id: TaskId,
    name: String,
    is_idle: bool,
    is_init: bool,

    entry: Option<*mut dyn FnOnce()>,
    state: AtomicU8,
    priority: AtomicUsize,

    in_wait_queue: AtomicBool,
    #[cfg(feature = "irq")]
    in_timer_list: AtomicBool,

    #[cfg(feature = "preempt")]
    need_resched: AtomicBool,
    #[cfg(feature = "preempt")]
    preempt_disable_count: AtomicUsize,

    exit_code: AtomicI32,
    pub(crate) wait_for_exit: WaitQueue,

    kstack: Option<TaskStack>,
    ctx: UnsafeCell<TaskContext>,

    #[cfg(feature = "tls")]
    tls: TlsArea,

    
    executor_ra: AtomicUsize,
    executor_sp: AtomicUsize,
    ret_context: UnsafeCell<TaskContext>,
}

impl TaskId {
    fn new() -> Self {
        static ID_COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(ID_COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Convert the task ID to a `u64`.
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

impl From<u8> for TaskState {
    #[inline]
    fn from(state: u8) -> Self {
        match state {
            1 => Self::Running,
            2 => Self::Ready,
            3 => Self::Blocked,
            4 => Self::Exited,
            _ => unreachable!(),
        }
    }
}

unsafe impl Send for SyncInner {}
unsafe impl Sync for SyncInner {}

pub(crate) trait UnpinFuture {
    type Output;
    fn poll(&self, cx: &mut Context<'_>) -> Poll<Self::Output>;
}

impl UnpinFuture for SyncInner {
    type Output = i32;
    
    fn poll(&self, cx: &mut Context<'_>) -> Poll<Self::Output> {
        info!("into poll");
        self.set_state(TaskState::Running);
        unsafe {
            let old_ctx = self.ret_ctx_mut_ptr().as_mut().unwrap();
            let new_ctx = self.ctx_mut_ptr().as_ref().unwrap();
            old_ctx.switch_to_receive_exit_value(new_ctx)
        }
    }
}

pub trait Downcast {
    fn to_task_inner(&self) -> Option<&SyncInner>;

    fn to_task_inner_mut(&mut self) -> Option<&mut SyncInner>;

    fn to_async_task_inner(&self) -> Option<&AsyncInner>;

    fn to_async_task_inner_mut(&mut self) -> Option<&mut AsyncInner>;
}

impl Downcast for SyncInner {
    fn to_task_inner(&self) -> Option<&SyncInner> {
        Some(self)
    }

    fn to_task_inner_mut(&mut self) -> Option<&mut SyncInner> {
        Some(self)
    }

    fn to_async_task_inner(&self) -> Option<&AsyncInner> {
        None
    }

    fn to_async_task_inner_mut(&mut self) -> Option<&mut AsyncInner> {
        None
    }
}

impl Downcast for AsyncInner {
    fn to_task_inner(&self) -> Option<&SyncInner> {
        None
    }

    fn to_task_inner_mut(&mut self) -> Option<&mut SyncInner> {
        None
    }

    fn to_async_task_inner(&self) -> Option<&AsyncInner> {
        Some(self)
    }

    fn to_async_task_inner_mut(&mut self) -> Option<&mut AsyncInner> {
        Some(self)
    }
}

pub trait TaskInfo {
    /// Gets the ID of the task.
    fn id(&self) -> TaskId;

    /// Gets the name of the task.
    fn name(&self) -> &str;

    /// Get a combined string of the task ID and name.
    fn id_name(&self) -> alloc::string::String;

    fn state(&self) -> TaskState;

    fn set_state(&self, state: TaskState);

    fn is_running(&self) -> bool;

    fn is_ready(&self) -> bool;

    fn is_blocked(&self) -> bool;

    fn is_exited(&self) -> bool;

    fn get_priority(&self) -> usize;

    fn set_priority(&self, priority: usize);

    fn is_init(&self) -> bool;

    fn is_idle(&self) -> bool;

    fn in_wait_queue(&self) -> bool;

    fn set_in_wait_queue(&self, in_wait_queue: bool);

    #[cfg(feature = "irq")]
    fn in_timer_list(&self) -> bool;

    #[cfg(feature = "irq")]
    fn set_in_timer_list(&self, in_timer_list: bool);

    fn is_async(&self) -> bool;
}

impl TaskInfo for SyncInner {
    /// Gets the ID of the task.
    fn id(&self) -> TaskId {
        self.id
    }

    /// Gets the name of the task.
    fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Get a combined string of the task ID and name.
    fn id_name(&self) -> alloc::string::String {
        alloc::format!("Task({}, {:?})", self.id.as_u64(), self.name)
    }

    
    fn state(&self) -> TaskState {
        self.state.load(Ordering::Acquire).into()
    }

    
    fn set_state(&self, state: TaskState) {
        self.state.store(state as u8, Ordering::Release)
    }

    
    fn is_running(&self) -> bool {
        matches!(self.state(), TaskState::Running)
    }

    
    fn is_ready(&self) -> bool {
        matches!(self.state(), TaskState::Ready)
    }

    
    fn is_blocked(&self) -> bool {
        matches!(self.state(), TaskState::Blocked)
    }

    fn is_exited(&self) -> bool {
        matches!(self.state(), TaskState::Exited)
    }

    
    fn get_priority(&self) -> usize {
        self.priority.load(Ordering::Relaxed)
    }

    
    fn set_priority(&self, priority: usize) {
        self.priority.store(priority, Ordering::Relaxed);
    }

    
    fn is_init(&self) -> bool {
        self.is_init
    }

    
    fn is_idle(&self) -> bool {
        self.is_idle
    }

    fn in_wait_queue(&self) -> bool {
        self.in_wait_queue.load(Ordering::Acquire)
    }

    fn set_in_wait_queue(&self, in_wait_queue: bool) {
        self.in_wait_queue.store(in_wait_queue, Ordering::Release);
    }

    #[cfg(feature = "irq")]
    fn in_timer_list(&self) -> bool {
        self.in_timer_list.load(Ordering::Acquire)
    }

    #[cfg(feature = "irq")]
    fn set_in_timer_list(&self, in_timer_list: bool) {
        self.in_timer_list.store(in_timer_list, Ordering::Release);
    }
    
    fn is_async(&self) -> bool {
        false
    }
}

impl SyncInner {
    /// Wait for the task to exit, and return the exit code.
    ///
    /// It will return immediately if the task has already exited (but not dropped).
    pub fn join(&self) -> Option<i32> {
        if !self.is_exited() {
            self.wait_for_exit.wait();
        }
        Some(self.exit_code.load(Ordering::Acquire))
    }
}

// private methods
impl SyncInner {
    fn new_common(id: TaskId, name: String) -> Self {
        Self {
            id,
            name,
            is_idle: false,
            is_init: false,
            entry: None,
            state: AtomicU8::new(TaskState::Ready as u8),
            priority: AtomicUsize::new(1),
            in_wait_queue: AtomicBool::new(false),
            #[cfg(feature = "irq")]
            in_timer_list: AtomicBool::new(false),
            #[cfg(feature = "preempt")]
            need_resched: AtomicBool::new(false),
            #[cfg(feature = "preempt")]
            preempt_disable_count: AtomicUsize::new(0),
            exit_code: AtomicI32::new(0),
            wait_for_exit: WaitQueue::new(),
            kstack: None,
            ctx: UnsafeCell::new(TaskContext::new()),
            #[cfg(feature = "tls")]
            tls: TlsArea::alloc(),
            
            executor_ra: AtomicUsize::new(0),
            executor_sp: AtomicUsize::new(0),
            ret_context: UnsafeCell::new(TaskContext::new()),
        }
    }

    /// Create a new task with the given entry function and stack size.
    pub(crate) fn new<F>(entry: F, name: String, stack_size: usize) -> AxTaskRef
    where
        F: FnOnce() + Send + 'static,
    {
        let mut t = Self::new_common(TaskId::new(), name);
        debug!("new task: {}", t.id_name());
        let kstack = TaskStack::alloc(align_up_4k(stack_size));

        #[cfg(feature = "tls")]
        let tls = VirtAddr::from(t.tls.tls_ptr() as usize);
        #[cfg(not(feature = "tls"))]
        let tls = VirtAddr::from(0);

        t.entry = Some(Box::into_raw(Box::new(entry)));
        t.ctx.get_mut().init(task_entry as usize, kstack.top(), tls);
        t.kstack = Some(kstack);
        if t.name == "idle" {
            t.is_idle = true;
        }
        Arc::new(AxTask::new_sync(t))
    }

    /// Create a new task with the given entry function and stack size.
    pub(crate) fn new_init<F>(entry: F, name: String, stack_size: usize) -> AxTaskRef
    where
        F: FnOnce() + Send + 'static,
    {
        let mut t = Self::new_common(TaskId::new(), name);
        t.is_init = true;
        debug!("new task: {}", t.id_name());
        let kstack = TaskStack::alloc(align_up_4k(stack_size));

        #[cfg(feature = "tls")]
        let tls = VirtAddr::from(t.tls.tls_ptr() as usize);
        #[cfg(not(feature = "tls"))]
        let tls = VirtAddr::from(0);

        t.entry = Some(Box::into_raw(Box::new(entry)));
        t.ctx.get_mut().init(task_entry as usize, kstack.top(), tls);
        t.kstack = Some(kstack);
        if t.name == "idle" {
            t.is_idle = true;
        }
        Arc::new(AxTask::new_sync(t))
    }

    /// Creates an "init task" using the current CPU states, to use as the
    /// current task.
    ///
    /// As it is the current task, no other task can switch to it until it
    /// switches out.
    ///
    /// And there is no need to set the `entry`, `kstack` or `tls` fields, as
    /// they will be filled automatically when the task is switches out.
    // pub(crate) fn new_init(name: String) -> AxTaskRef {
    //     let mut t = Self::new_common(TaskId::new(), name);
    //     t.is_init = true;
    //     if t.name == "idle" {
    //         t.is_idle = true;
    //     }
    //     Arc::new(AxTask::new_sync(t))
    // }

    #[inline]
    #[cfg(feature = "preempt")]
    pub(crate) fn set_preempt_pending(&self, pending: bool) {
        self.need_resched.store(pending, Ordering::Release)
    }

    #[inline]
    #[cfg(feature = "preempt")]
    pub(crate) fn can_preempt(&self, current_disable_count: usize) -> bool {
        self.preempt_disable_count.load(Ordering::Acquire) == current_disable_count
    }

    #[inline]
    #[cfg(feature = "preempt")]
    pub(crate) fn disable_preempt(&self) {
        self.preempt_disable_count.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    #[cfg(feature = "preempt")]
    pub(crate) fn enable_preempt(&self, resched: bool) {
        if self.preempt_disable_count.fetch_sub(1, Ordering::Relaxed) == 1 && resched {
            // If current task is pending to be preempted, do rescheduling.
            Self::current_check_preempt_pending();
        }
    }

    #[cfg(feature = "preempt")]
    fn current_check_preempt_pending() {
        let curr = crate::current();
        if curr.need_resched.load(Ordering::Acquire) && curr.can_preempt(0) {
            let mut rq = crate::RUN_QUEUE.lock();
            if curr.need_resched.load(Ordering::Acquire) {
                rq.preempt_resched();
            }
        }
    }

    // pub(crate) fn notify_exit(&self, exit_code: i32) {
    //     self.exit_code.store(exit_code, Ordering::Release);
    //     self.wait_for_exit.notify_all_locked(false);
    // }

    #[inline]
    pub(crate) const unsafe fn ctx_mut_ptr(&self) -> *mut TaskContext {
        self.ctx.get()
    }

    #[inline]
    pub(crate) const unsafe fn ret_ctx_mut_ptr(&self) -> *mut TaskContext {
        self.ret_context.get()
    }
}

impl fmt::Debug for SyncInner {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("TaskInner")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("state", &self.state())
            .finish()
    }
}

impl Drop for SyncInner {
    fn drop(&mut self) {
        debug!("task drop: {}", self.id_name());
    }
}

/// The inner coroutine structure.
pub struct AsyncInner {
    id: TaskId,
    name: String,
    is_idle: bool,
    is_init: bool,
    state: AtomicU8,
    priority: AtomicUsize,

    in_wait_queue: AtomicBool,
    #[cfg(feature = "irq")]
    in_timer_list: AtomicBool,

    pub(crate) exit_code: AtomicI32,
    pub(crate) wait_for_exit: WaitQueue,

    fut: UnsafeCell<Pin<Box<dyn Future<Output = i32> + Send + Sync>>>,
}

impl AsyncInner {
    /// Wait for the task to exit, and return the exit code.
    ///
    /// It will return immediately if the task has already exited (but not dropped).
    pub fn join(&self) -> Option<i32> {
        if !self.is_exited() {
            self.wait_for_exit.wait();
        }
        Some(self.exit_code.load(Ordering::Acquire))
    }
}

unsafe impl Send for AsyncInner { }
unsafe impl Sync for AsyncInner { }

impl UnpinFuture for AsyncInner {
    type Output = i32;

    fn poll(&self, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.set_state(TaskState::Running);
        let res = unsafe { self.fut.get().as_mut().unwrap().as_mut().poll(cx) };
        match &res {
            Poll::Pending => {
                self.set_state(TaskState::Blocked);
            },
            Poll::Ready(value) => {
                self.exit_code.store(*value, Ordering::Release);
                self.set_state(TaskState::Exited);
            }
        }
        res
    }
}

impl TaskInfo for AsyncInner {
    /// Gets the ID of the task.
    fn id(&self) -> TaskId {
        self.id
    }

    /// Gets the name of the task.
    fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Get a combined string of the task ID and name.
    fn id_name(&self) -> alloc::string::String {
        alloc::format!("Task({}, {:?})", self.id.as_u64(), self.name)
    }

    
    fn state(&self) -> TaskState {
        self.state.load(Ordering::Acquire).into()
    }

    
    fn set_state(&self, state: TaskState) {
        self.state.store(state as u8, Ordering::Release)
    }

    
    fn is_running(&self) -> bool {
        matches!(self.state(), TaskState::Running)
    }

    
    fn is_ready(&self) -> bool {
        matches!(self.state(), TaskState::Ready)
    }

    
    fn is_blocked(&self) -> bool {
        matches!(self.state(), TaskState::Blocked)
    }

    fn is_exited(&self) -> bool {
        matches!(self.state(), TaskState::Exited)
    }

    fn get_priority(&self) -> usize {
        self.priority.load(Ordering::Relaxed)
    }

    
    fn set_priority(&self, priority: usize) {
        self.priority.store(priority, Ordering::Relaxed);
    }

    
    fn is_init(&self) -> bool {
        self.is_init
    }

    
    fn is_idle(&self) -> bool {
        self.is_idle
    }

    
    fn in_wait_queue(&self) -> bool {
        self.in_wait_queue.load(Ordering::Acquire)
    }

    
    fn set_in_wait_queue(&self, in_wait_queue: bool) {
        self.in_wait_queue.store(in_wait_queue, Ordering::Release);
    }

    
    #[cfg(feature = "irq")]
    fn in_timer_list(&self) -> bool {
        self.in_timer_list.load(Ordering::Acquire)
    }

    
    #[cfg(feature = "irq")]
    fn set_in_timer_list(&self, in_timer_list: bool) {
        self.in_timer_list.store(in_timer_list, Ordering::Release);
    }
    
    
    fn is_async(&self) -> bool {
        true
    }
}

impl AsyncInner {
    /// Create a new task with the given entry function and stack size.
    pub(crate) fn new<F>(fut: F, name: String) -> AxTaskRef
    where
        F: Future<Output = i32> + Send + Sync + 'static
    {
        let mut t = Self {
            id: TaskId::new(),
            name,
            is_idle: false,
            is_init: false,
            state: AtomicU8::new(TaskState::Ready as u8),
            priority: AtomicUsize::new(1),
            in_wait_queue: AtomicBool::new(false),
            #[cfg(feature = "irq")]
            in_timer_list: AtomicBool::new(false),
            exit_code: AtomicI32::new(0),
            wait_for_exit: WaitQueue::new(),
            fut: UnsafeCell::new(Box::pin(fut))
        };
        Arc::new(AxTask::new_async(t))
    }
}

pub(crate) struct TaskStack {
    ptr: NonNull<u8>,
    layout: Layout,
}

impl TaskStack {
    pub fn alloc(size: usize) -> Self {
        let layout = Layout::from_size_align(size, 16).unwrap();
        Self {
            ptr: NonNull::new(unsafe { alloc::alloc::alloc(layout) }).unwrap(),
            layout,
        }
    }

    pub const fn top(&self) -> VirtAddr {
        unsafe { core::mem::transmute(self.ptr.as_ptr().add(self.layout.size())) }
    }
}

impl Drop for TaskStack {
    fn drop(&mut self) {
        unsafe { alloc::alloc::dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

use core::mem::{self, ManuallyDrop};

/// A wrapper of [`AxTaskRef`] as the current task.
/// `None` indecates current task is executor.
pub struct CurrentTask(RefCell<Option<ManuallyDrop<AxTaskRef>>>);

impl CurrentTask {
    // pub(crate) fn try_get() -> Option<Self> {
    //     let ptr: *const super::AxTask = axhal::cpu::current_task_ptr();
    //     if !ptr.is_null() {
    //         Some(Self(unsafe { ManuallyDrop::new(AxTaskRef::from_raw(ptr)) }))
    //     } else {
    //         None
    //     }
    // }

    // pub(crate) fn get() -> Self {
    //     Self::try_get().expect("current task is uninitialized")
    // }

    pub fn new() -> Self {
        Self {
            0: RefCell::new(None),
        }
    } 

    /// Converts [`CurrentTask`] to [`AxTaskRef`].
    // pub fn as_task_ref(&self) -> Option<AxTaskRef> {
    //     self.0.map(|a| { ManuallyDrop::into_inner(a) })
    // }

    pub(crate) fn get_clone(&self) -> Option<AxTaskRef> {
        self.0.borrow().as_deref().map(|a| { a.clone() })
    }

    pub(crate) fn ptr_eq(&self, other: &AxTaskRef) -> bool {
        if self.0.borrow().is_none() { false }
        else {
            Arc::ptr_eq(self.0.borrow().as_ref().unwrap(), other)
        }
    }

    // pub(crate) unsafe fn init_current(init_task: AxTaskRef) {
    //     #[cfg(feature = "tls")]
    //     axhal::arch::write_thread_pointer(init_task.tls.tls_ptr() as usize);
    //     let ptr = Arc::into_raw(init_task);
    //     axhal::cpu::set_current_task_ptr(ptr);
    // }

    pub(crate) unsafe fn set_current(&self, next: Option<AxTaskRef>) {
        let old = mem::replace(self.0.borrow_mut().deref_mut(),  next.map(|a| { ManuallyDrop::new(a) }));
        if let Some(md) = old {
            ManuallyDrop::into_inner(md); // `call Arc::drop()` to decrease prev task reference count.
        }
        // self.0 = next.map(|a| { ManuallyDrop::new(a) });
        // axhal::cpu::set_current_task_ptr(ptr);
    }
}

// impl Deref for CurrentTask {
//     type Target = AxTask;
//     fn deref(&self) -> &Self::Target {
//         self.0.borrow().as_deref().unwrap().clone().deref()
//     }
// }

unsafe impl Send for CurrentTask { }
unsafe impl Sync for CurrentTask { }

/// Only used for initialization
impl Clone for CurrentTask {
    fn clone(&self) -> Self {
        assert!(self.0.borrow().is_none());
        Self::new()
    }
}

extern "C" fn task_entry() -> ! {
    // // release the lock that was implicitly held across the reschedule
    // unsafe { crate::RUN_QUEUE.force_unlock() };
    #[cfg(feature = "irq")]
    axhal::arch::enable_irqs();
    let task = crate::current().unwrap();
    assert!(!task.is_async());
    // let task_inner = unsafe { &*(task.inner.as_ref() as *const dyn AbsTaskInner as *const () as *const TaskInner) };
    let task_inner = task.task_inner.to_task_inner().unwrap();
    if let Some(entry) = task_inner.entry {
        unsafe { Box::from_raw(entry)() };
    }
    crate::exit(0);
}
