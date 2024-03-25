use core::{cell::{RefCell, UnsafeCell}, hint::spin_loop, task::Poll};
use ats_intc::AtsIntc;
use axhal::arch::TaskContext;
use lazy_init::LazyInit;
use spinlock::SpinNoIrq;
use crate::{task::{AbsTaskInner, AxTask, TaskStack}, AxTaskRef, CurrentTask, WaitQueue};
use core::task::{ Context, Waker };
use alloc::{collections::VecDeque, sync::Arc};
use core::arch::asm;
use memory_addr::align_up_4k;

// pub(crate) static ATS_DRIVER: Lazy<AtsDriver> = Lazy::new(| |{ AtsDriver::new(0xffff_ffc0_0f00_0000) });
pub(crate) static ATS_DRIVER: LazyInit<AtsIntc> = LazyInit::new();
pub(crate) const PROCESS_ID: usize = 0;

// scheduler and executor
// pub(crate) static ATS_EXECUTOR: Lazy<Ats> = Lazy::new(| |{ Ats::new(PROCESS_ID) });
// pub(crate) static CURRENT_TASK: Lazy<RefCell<CurrentTask>> = Lazy::new(| |{ RefCell::new(CurrentTask::new()) });
pub(crate) static ATS_EXECUTOR: LazyInit<Ats> = LazyInit::new();
pub(crate) static CURRENT_TASK: LazyInit<CurrentTask> = LazyInit::new();

// TODO: per-CPU
pub(crate) static EXITED_TASKS: SpinNoIrq<VecDeque<AxTaskRef>> = SpinNoIrq::new(VecDeque::new());

pub(crate) static WAIT_FOR_EXIT: WaitQueue = WaitQueue::new();

pub struct Ats {
    process_id: usize,
    stack: TaskStack,
    ctx: UnsafeCell<TaskContext>,
}

unsafe impl Send for Ats { }
unsafe impl Sync for Ats { }

impl Ats {
    pub(crate) fn new(process_id: usize) -> Self {
        Self {
            process_id,
            stack: TaskStack::alloc(align_up_4k(axconfig::TASK_STACK_SIZE)),
            ctx: UnsafeCell::new(TaskContext::new()),
        }
    }

    pub(crate) fn run(&self) -> ! {
        loop {
            info!("  into Ats::run");
            let task = ATS_DRIVER.ps_fetch();
            info!("  after ftask");
            match task {
                Some(task_ref) => {
                    unsafe {
                        info!("  ftask: Some");
                        let task: Arc<AxTask> = Arc::from_raw(task_ref as *const AxTask);
                        info!("  fetch task: {}.", task.id_name());
                        CURRENT_TASK.set_current(Some(task.clone()));
                        match task.poll(&mut Context::from_waker(&Waker::from(task.clone()))) { 
                            Poll::Ready(value) => {
                                info!("  task return {}.", value);
                            },
                            Poll::Pending => {
                                info!("  task not finished.");
                            },
                        }
                        CURRENT_TASK.set_current(None);
                    }
                },
                None => {
                    info!("  ftask: None");
                    // spin_loop();
                    axhal::misc::terminate();
                }
            }
        }
    }
}