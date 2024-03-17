use alloc::task::Wake;
use alloc::{boxed::Box, string::String, sync::Arc};
use core::future::Future;
use core::ops::Deref;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use core::task::{Context, Poll};
use core::{alloc::Layout, cell::UnsafeCell, fmt, ptr::NonNull};

use core::arch::asm;
use ats_intc::AtsDriver;

#[cfg(feature = "preempt")]
use core::sync::atomic::AtomicUsize;

#[cfg(feature = "tls")]
use axhal::tls::TlsArea;

use axhal::arch::TaskContext;
use memory_addr::{align_up_4k, VirtAddr, PAGE_SIZE_4K};

use crate::ats::PROCESS_ID;
use crate::ats::{EXITED_TASKS, WAIT_FOR_EXIT};
use crate::{AxTask, AxTaskRef, WaitQueue};
use crate::ats::ATS_DRIVER;

pub trait AbsTaskInner: UnpinFuture<Output = i32> + TaskInfo + Send + Sync {
    fn to_waker(self: Arc<Self>) -> AbsTaskWaker {
        AbsTaskWaker {
            inner: Arc::clone(self),
        }
    }
}
impl AbsTaskInner for TaskInner { }
impl AbsTaskInner for AsyncTaskInner { }

pub struct AbsTaskWaker {
    inner: AxTaskRef,
}

impl Wake for AbsTaskWaker {
    fn wake(self: Arc<Self>) {
        self.inner.set_state(TaskState::Ready);
        let task_ref = self.inner.as_ref() as *const AbsTaskInner as *const _ as usize;
        ATS_DRIVER.stask(task_ref, PROCESS_ID, self.inner.get_priority());
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
pub struct TaskInner {
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
    wait_for_exit: WaitQueue,

    kstack: Option<TaskStack>,
    ctx: UnsafeCell<TaskContext>,

    #[cfg(feature = "tls")]
    tls: TlsArea,

    
    executor_ra: AtomicUsize,
    executor_sp: AtomicUsize,
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

unsafe impl Send for TaskInner {}
unsafe impl Sync for TaskInner {}

pub(crate) trait UnpinFuture {
    type Output;
    fn poll(&self, cx: &mut Context<'_>) -> Poll<Self::Output>;
}

impl UnpinFuture for TaskInner {
    type Output = i32;
    
    fn poll(&self, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut ra: usize = 0;
        let mut sp: usize = 0;
        unsafe {
            asm! {
                "
                    MOV     t0, ra
                    MOV     t1, sp
                ",
                out("t0") ra,
                out("t1") sp,
            };
        }
        self.executor_ra.store(ra, Ordering::Relaxed);
        self.executor_sp.store(sp, Ordering::Relaxed);

        unsafe {
            self.ctx_mut_ptr().as_mut().unwrap().context_load();
            asm! {
                "RET"
            }
        }

        return Poll::Ready(-1); // unreachable
    }
}

impl Wake for TaskInner {
    fn wake(self: Arc<Self>) {
        self.set_state(TaskState::Ready);
        let task_ref = self.as_ref() as *const TaskInner as usize;
        ATS_DRIVER.stask(task_ref, PROCESS_ID, self.get_priority());
    }
}

// related to async
impl TaskInner {
    pub fn yield_self(&self) {
        assert!(self.is_running());
        self.set_state(TaskState::Ready);

        // TODO: Call Waker

        unsafe {
            self.ctx_mut_ptr().as_mut().unwrap().context_store();
            
            let ra = self.executor_ra.load(Ordering::Relaxed);
            let sp = self.executor_sp.load(Ordering::Relaxed);
            asm! {
                "
                    MOV     ra, t0
                    MOV     sp, t1
                    // return Poll::Pending to Executor
                    LI      a0, 1
                    RET
                ",
                in("t0") ra,
                in("t1") sp,
            }
        }
    }

    pub fn exit(&self, exit_code: i32) -> ! {
        assert!(self.is_running());
        assert!(!self.is_idle());
        if self.is_init() {
            EXITED_TASKS.lock().clear();
            axhal::misc::terminate();
        } else {
            self.set_state(TaskState::Exited);
            self.exit_code.store(exit_code, Ordering::Release);
            self.wait_for_exit.notify_all_locked(false);
            EXITED_TASKS.lock().push_back(unsafe { Arc::from_raw(self as *const Self) });
            WAIT_FOR_EXIT.notify_one_locked(false);
            
            unsafe {
                let ra = self.executor_ra.load(Ordering::Relaxed);
                let sp = self.executor_sp.load(Ordering::Relaxed);
                asm! {
                    "
                        MOV     ra, t0
                        MOV     sp, t1
                        // return Poll::Ready(exit_code) to Executor
                        LI      a0, 0
                        MOV     a1, t2
                        RET
                    ",
                    in("t0") ra,
                    in("t1") sp,
                    in("t2") exit_code,
                }
            }

            unreachable!();
        }
    }
}

pub trait TaskInfo {
    /// Gets the ID of the task.
    fn id(&self) -> TaskId;

    /// Gets the name of the task.
    fn name(&self) -> &str;

    /// Get a combined string of the task ID and name.
    fn id_name(&self) -> alloc::string::String;

    #[inline]
    fn state(&self) -> TaskState;

    #[inline]
    fn set_state(&self, state: TaskState);

    #[inline]
    fn is_running(&self) -> bool;

    #[inline]
    fn is_ready(&self) -> bool;

    #[inline]
    fn is_blocked(&self) -> bool;

    #[inline]
    fn get_priority(&self) -> usize;

    #[inline]
    fn set_priority(&self, priority: usize);

    #[inline]
    fn is_init(&self) -> bool;

    #[inline]
    fn is_idle(&self) -> bool;

    #[inline]
    fn in_wait_queue(&self) -> bool;

    #[inline]
    fn set_in_wait_queue(&self, in_wait_queue: bool);

    #[inline]
    #[cfg(feature = "irq")]
    fn in_timer_list(&self) -> bool;

    #[inline]
    #[cfg(feature = "irq")]
    fn set_in_timer_list(&self, in_timer_list: bool);

    #[inline]
    fn is_async(&self) -> bool;
}

impl TaskInfo for TaskInner {
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

    #[inline]
    fn state(&self) -> TaskState {
        self.state.load(Ordering::Acquire).into()
    }

    #[inline]
    fn set_state(&self, state: TaskState) {
        self.state.store(state as u8, Ordering::Release)
    }

    #[inline]
    fn is_running(&self) -> bool {
        matches!(self.state(), TaskState::Running)
    }

    #[inline]
    fn is_ready(&self) -> bool {
        matches!(self.state(), TaskState::Ready)
    }

    #[inline]
    fn is_blocked(&self) -> bool {
        matches!(self.state(), TaskState::Blocked)
    }

    #[inline]
    fn get_priority(&self) -> usize {
        self.priority.load(Ordering::Relaxed)
    }

    #[inline]
    fn set_priority(&self, priority: usize) {
        self.priority.store(priority, Ordering::Relaxed);
    }

    #[inline]
    fn is_init(&self) -> bool {
        self.is_init
    }

    #[inline]
    fn is_idle(&self) -> bool {
        self.is_idle
    }

    #[inline]
    fn in_wait_queue(&self) -> bool {
        self.in_wait_queue.load(Ordering::Acquire)
    }

    #[inline]
    fn set_in_wait_queue(&self, in_wait_queue: bool) {
        self.in_wait_queue.store(in_wait_queue, Ordering::Release);
    }

    #[inline]
    #[cfg(feature = "irq")]
    fn in_timer_list(&self) -> bool {
        self.in_timer_list.load(Ordering::Acquire)
    }

    #[inline]
    #[cfg(feature = "irq")]
    fn set_in_timer_list(&self, in_timer_list: bool) {
        self.in_timer_list.store(in_timer_list, Ordering::Release);
    }
    
    #[inline]
    fn is_async(&self) -> bool {
        false
    }
}

impl TaskInner {
    /// Wait for the task to exit, and return the exit code.
    ///
    /// It will return immediately if the task has already exited (but not dropped).
    pub fn join(&self) -> Option<i32> {
        self.wait_for_exit
            .wait_until(|| self.state() == TaskState::Exited);
        Some(self.exit_code.load(Ordering::Acquire))
    }
}

// private methods
impl TaskInner {
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
        Arc::new(t)
    }

    /// Creates an "init task" using the current CPU states, to use as the
    /// current task.
    ///
    /// As it is the current task, no other task can switch to it until it
    /// switches out.
    ///
    /// And there is no need to set the `entry`, `kstack` or `tls` fields, as
    /// they will be filled automatically when the task is switches out.
    pub(crate) fn new_init(name: String) -> AxTaskRef {
        let mut t = Self::new_common(TaskId::new(), name);
        t.is_init = true;
        if t.name == "idle" {
            t.is_idle = true;
        }
        Arc::new(t)
    }

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
}

impl fmt::Debug for TaskInner {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("TaskInner")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("state", &self.state())
            .finish()
    }
}

impl Drop for TaskInner {
    fn drop(&mut self) {
        debug!("task drop: {}", self.id_name());
    }
}

/// The inner coroutine structure.
pub struct AsyncTaskInner {
    id: TaskId,
    name: String,
    is_idle: bool,
    is_init: bool,
    state: AtomicU8,
    priority: AtomicUsize,

    in_wait_queue: AtomicBool,
    #[cfg(feature = "irq")]
    in_timer_list: AtomicBool,

    wait_for_exit: WaitQueue,

    fut: UnsafeCell<Pin<Box<dyn Future<Output = i32> + Send + Sync>>>,
}

unsafe impl Send for AsyncTaskInner { }
unsafe impl Sync for AsyncTaskInner { }

impl UnpinFuture for AsyncTaskInner {
    type Output = i32;

    fn poll(&self, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.set_state(TaskState::Running);
        let res = unsafe { self.fut.get().as_mut().unwrap().as_mut().poll(cx) };
        self.set_state(TaskState::Blocked);
        res
    }
}

impl Wake for AsyncTaskInner {
    fn wake(self: Arc<Self>) {
        self.set_state(TaskState::Ready);
        let task_ref = self.as_ref() as *const AsyncTaskInner as usize;
        ATS_DRIVER.stask(task_ref, PROCESS_ID, self.get_priority());
    }
}

impl TaskInfo for AsyncTaskInner {
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

    #[inline]
    fn state(&self) -> TaskState {
        self.state.load(Ordering::Acquire).into()
    }

    #[inline]
    fn set_state(&self, state: TaskState) {
        self.state.store(state as u8, Ordering::Release)
    }

    #[inline]
    fn is_running(&self) -> bool {
        matches!(self.state(), TaskState::Running)
    }

    #[inline]
    fn is_ready(&self) -> bool {
        matches!(self.state(), TaskState::Ready)
    }

    #[inline]
    fn is_blocked(&self) -> bool {
        matches!(self.state(), TaskState::Blocked)
    }

    #[inline]
    fn get_priority(&self) -> usize {
        self.priority.load(Ordering::Relaxed)
    }

    #[inline]
    fn set_priority(&self, priority: usize) {
        self.priority.store(priority, Ordering::Relaxed);
    }

    #[inline]
    fn is_init(&self) -> bool {
        self.is_init
    }

    #[inline]
    fn is_idle(&self) -> bool {
        self.is_idle
    }

    #[inline]
    fn in_wait_queue(&self) -> bool {
        self.in_wait_queue.load(Ordering::Acquire)
    }

    #[inline]
    fn set_in_wait_queue(&self, in_wait_queue: bool) {
        self.in_wait_queue.store(in_wait_queue, Ordering::Release);
    }

    #[inline]
    #[cfg(feature = "irq")]
    fn in_timer_list(&self) -> bool {
        self.in_timer_list.load(Ordering::Acquire)
    }

    #[inline]
    #[cfg(feature = "irq")]
    fn set_in_timer_list(&self, in_timer_list: bool) {
        self.in_timer_list.store(in_timer_list, Ordering::Release);
    }
    
    #[inline]
    fn is_async(&self) -> bool {
        true
    }
}

impl AsyncTaskInner {
    /// Create a new task with the given entry function and stack size.
    pub(crate) fn new<F>(fut: F, name: String) -> Arc<dyn AbsTaskInner>
    where
        F: Future<Output = i32> + Send + Sync
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
            wait_for_exit: WaitQueue::new(),
            fut: UnsafeCell::new(Box::pin(fut))
        };
        Arc::new(t)
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
pub struct CurrentTask(Option<ManuallyDrop<AxTaskRef>>);

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
            0: None
        }
    } 

    /// Converts [`CurrentTask`] to [`AxTaskRef`].
    pub fn as_task_ref(&self) -> Option<AxTaskRef> {
        self.0.map(|a| { ManuallyDrop::into_inner(a) })
    }

    pub(crate) fn clone(&self) -> Option<AxTaskRef> {
        self.0.map(|a| { ManuallyDrop::into_inner(a).clone() })
    }

    pub(crate) fn ptr_eq(&self, other: &AxTaskRef) -> bool {
        if self.0.is_none() { false }
        else {
            Arc::ptr_eq(&self.0.unwrap(), other)
        }
    }

    // pub(crate) unsafe fn init_current(init_task: AxTaskRef) {
    //     #[cfg(feature = "tls")]
    //     axhal::arch::write_thread_pointer(init_task.tls.tls_ptr() as usize);
    //     let ptr = Arc::into_raw(init_task);
    //     axhal::cpu::set_current_task_ptr(ptr);
    // }

    pub(crate) unsafe fn set_current(&mut self, next: Option<AxTaskRef>) {
        let old = mem::replace(self, CurrentTask {0: next.map(|a| { ManuallyDrop::new(a) })});
        let Self(opt) = old;
        if let Some(arc) = opt {
            ManuallyDrop::into_inner(arc); // `call Arc::drop()` to decrease prev task reference count.
        }
        // self.0 = next.map(|a| { ManuallyDrop::new(a) });
        // axhal::cpu::set_current_task_ptr(ptr);
    }
}

impl Deref for CurrentTask {
    type Target = dyn AbsTaskInner;
    fn deref(&self) -> &Self::Target {
        self.0.unwrap().deref()
    }
}

extern "C" fn task_entry() -> ! {
    // // release the lock that was implicitly held across the reschedule
    // unsafe { crate::RUN_QUEUE.force_unlock() };
    #[cfg(feature = "irq")]
    axhal::arch::enable_irqs();
    let task = crate::current().as_task_ref().unwrap();
    assert!(!task.is_async());
    let inner = task.as_ref() as *const dyn AbsTaskInner as *const _ as *const TaskInner as &TaskInner;
    if let Some(entry) = inner.entry {
        unsafe { Box::from_raw(entry)() };
    }
    crate::exit(0);
}
