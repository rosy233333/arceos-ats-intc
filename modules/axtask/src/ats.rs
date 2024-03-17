use core::{hint::spin_loop, task::Poll};
use ats_intc::AtsDriver;
use spinlock::SpinNoIrq;
use crate::{task::{AbsTaskInner, TaskStack}, AxTaskRef, CurrentTask, WaitQueue};
use core::task::{ Context, Waker };
use alloc::{collections::VecDeque, sync::Arc};

pub(crate) static ATS_DRIVER: AtsDriver = AtsDriver::new(0xffff_ffc0_0f00_0000);
pub(crate) const PROCESS_ID: usize = 0;

// scheduler and executor
pub(crate) static ATS_EXECUTOR: Ats = Ats::new(PROCESS_ID);
pub(crate) static CURRENT_TASK: CurrentTask = CurrentTask::new();

// TODO: per-CPU
pub(crate) static EXITED_TASKS: SpinNoIrq<VecDeque<AxTaskRef>> = SpinNoIrq::new(VecDeque::new());

pub(crate) static WAIT_FOR_EXIT: WaitQueue = WaitQueue::new();

pub struct Ats {
    process_id: usize,
    stack: TaskStack,
}

unsafe impl Sync for Ats { }

impl Ats {
    fn new(process_id: usize) -> Self {
        Self {
            process_id,
            stack: TaskStack::alloc(axconfig::TASK_STACK_SIZE),
        }
    }

    pub(crate) fn run(&self) -> ! {
        loop {
            let task = ATS_DRIVER.ftask(self.process_id);
            match task {
                Some(task_ref) => {
                    unsafe {
                        let task: Arc<dyn AbsTaskInner> = Arc::from_raw(task_ref as *const ());
                        CURRENT_TASK.set_current(Some(task));
                        match task.poll(&mut Context::from_waker(&Waker::from(Arc::new(task.to_waker())))) { 
                            Poll::Ready(value) => {
                                info!("  task return {}.", value);
                            },
                            Poll::Pending => {

                            },
                        }
                        CURRENT_TASK.set_current(None);
                    }
                },
                None => {
                    spin_loop();
                }
            }
        }
    }
}