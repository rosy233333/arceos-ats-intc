use core::{cell::{RefCell, UnsafeCell}, hint::spin_loop, task::Poll};
use ats_intc::AtsIntc;
use axhal::{arch::TaskContext, cpu::this_cpu_id};
use lazy_init::LazyInit;
use spinlock::SpinNoIrq;
use crate::{spawn_async, task::{AbsTaskInner, AsyncTaskInner, AxTask, TaskStack}, AxTaskRef, CurrentTask, WaitQueue};
use core::task::{ Context, Waker };
use alloc::{collections::VecDeque, sync::Arc};
use core::arch::asm;
use memory_addr::align_up_4k;
use alloc::vec::Vec;

pub(crate) static DRIVER_LOCK: SpinNoIrq<usize> = SpinNoIrq::new(0);

pub(crate) static GLOBAL_ATS_DRIVER: SpinNoIrq<LazyInit<AtsIntc>> = SpinNoIrq::new(LazyInit::new());

#[percpu::def_percpu]
pub(crate) static ATS_DRIVER: LazyInit<AtsIntc> = LazyInit::new();

pub(crate) const PROCESS_ID: usize = 0;

// scheduler and executor
#[percpu::def_percpu]
pub(crate) static ATS_EXECUTORS: LazyInit<Ats> = LazyInit::new();
#[percpu::def_percpu]
pub(crate) static CURRENT_TASKS: LazyInit<CurrentTask> = LazyInit::new();

// TODO: per-CPU
pub(crate) static EXITED_TASKS: SpinNoIrq<VecDeque<AxTaskRef>> = SpinNoIrq::new(VecDeque::new());

pub(crate) static WAIT_FOR_EXIT: WaitQueue = WaitQueue::new();

pub struct Ats {
    process_id: usize,
    stack: TaskStack,
    pub cpu_id: usize,
    ctx: UnsafeCell<TaskContext>,
}

unsafe impl Send for Ats { }
unsafe impl Sync for Ats { }

impl Ats {
    pub(crate) fn new(process_id: usize) -> Self {
        Self {
            process_id,
            stack: TaskStack::alloc(align_up_4k(axconfig::TASK_STACK_SIZE)),
            cpu_id: this_cpu_id(), // uninitialized
            ctx: UnsafeCell::new(TaskContext::new()),
        }
    }

    pub(crate) fn run(&self) -> ! {
        loop {
            let cpu_id: usize = self.cpu_id;
            let current_cpu_id = this_cpu_id();
            assert!(current_cpu_id == cpu_id);
            info!("  into Ats::run");

            let ats_task = unsafe {
                let driver = ATS_DRIVER.current_ref_raw();
                // let driver = GLOBAL_ATS_DRIVER.lock();
                driver.ps_fetch()
            };
            // let _task = unsafe {
            //     let driver = GLOBAL_ATS_DRIVER.lock();
            //     // error!("ps_fetch");
            //     driver.ps_fetch()
            // };
            // let ats_task = Some(AsyncTaskInner::new(async {
            //     spawn_async(async {
            //         0
            //     });
            //     0
            // }, "test".into()).into_task_ref());
            
            info!("  after ftask");
            match ats_task {
                Some(task_ref) => {
                        // error!("  ftask: Some");
                        let task: Arc<AxTask> = unsafe { AxTask::from_task_ref(task_ref) };
                        // error!("  fetch task: {}.", task.id_name());
                        unsafe {
                            // let ct_lock = CURRENT_TASKS.lock();
                            // ct_lock[cpu_id].set_current(Some(task.clone()));
                            CURRENT_TASKS.current_ref_raw().set_current(Some(task.clone()));
                        }
                        let poll_result = task.poll(&mut Context::from_waker(&Waker::from(task.clone())));
                        unsafe {
                            // let ct_lock = CURRENT_TASKS.lock();
                            // ct_lock[cpu_id].set_current(None);
                            CURRENT_TASKS.current_ref_raw().set_current(None);
                        }
                        match poll_result { 
                            Poll::Ready(value) => {
                                info!("  task return {}.", value);
                            },
                            Poll::Pending => {
                                info!("  task not finished.");
                            },
                        }
                },
                None => {
                    // info!("  ftask: None");
                    // spin_loop();
                    // axhal::misc::terminate();
                }
            }
        }
    }
}

/// Only used for initialization
impl Clone for Ats {
    fn clone(&self) -> Self {
        Self::new(self.process_id)
    }
}