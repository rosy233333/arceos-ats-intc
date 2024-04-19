﻿use core::{cell::{RefCell, UnsafeCell}, hint::spin_loop, task::Poll};
use ats_intc::AtsIntc;
use axhal::{arch::TaskContext, cpu::this_cpu_id};
use lazy_init::LazyInit;
use spinlock::SpinNoIrq;
use crate::{spawn_async, task::{AbsTaskInner, AxTask, TaskStack}, AxTaskRef, CurrentTask, WaitQueue};
use core::task::{ Context, Waker };
use alloc::{collections::VecDeque, sync::Arc};
use core::arch::asm;
use memory_addr::align_up_4k;
use alloc::vec::Vec;

pub(crate) static ATS_DRIVER: LazyInit<Arc<SpinNoIrq<AtsIntc>>> = LazyInit::new();
pub(crate) const PROCESS_ID: usize = 0;

// scheduler and executor
pub(crate) static ATS_EXECUTORS: LazyInit<Vec<Ats>> = LazyInit::new();
pub(crate) static CURRENT_TASKS: LazyInit<SpinNoIrq<Vec<CurrentTask>>> = LazyInit::new();

// TODO: per-CPU
pub(crate) static EXITED_TASKS: SpinNoIrq<VecDeque<AxTaskRef>> = SpinNoIrq::new(VecDeque::new());

pub(crate) static WAIT_FOR_EXIT: WaitQueue = WaitQueue::new();

pub struct Ats {
    process_id: usize,
    stack: TaskStack,
    pub cpu_id: LazyInit<usize>,
    ctx: UnsafeCell<TaskContext>,
}

unsafe impl Send for Ats { }
unsafe impl Sync for Ats { }

impl Ats {
    pub(crate) fn new(process_id: usize) -> Self {
        Self {
            process_id,
            stack: TaskStack::alloc(align_up_4k(axconfig::TASK_STACK_SIZE)),
            cpu_id: LazyInit::new(), // uninitialized
            ctx: UnsafeCell::new(TaskContext::new()),
        }
    }

    pub(crate) fn run(&self) -> ! {
        loop {
            let cpu_id: usize = *self.cpu_id;
            let current_cpu_id = this_cpu_id();
            assert!(current_cpu_id == cpu_id);
            // info!("  into Ats::run");
            let ats_task = {
                info!("  before lock");
                let driver_lock = ATS_DRIVER.lock();
                info!("  after lock");
                driver_lock.ps_fetch()
            };
            info!("  after ftask");
            match ats_task {
                Some(task_ref) => {
                        // error!("  ftask: Some");
                        let task: Arc<AxTask> = unsafe { AxTask::from_task_ref(task_ref) };
                        // error!("  fetch task: {}.", task.id_name());
                        // unsafe {
                        //     let ct_lock = CURRENT_TASKS.lock();
                        //     ct_lock[cpu_id].set_current(Some(task.clone()));
                        // }
                        // let poll_result = task.poll(&mut Context::from_waker(&Waker::from(task.clone())));
                        // unsafe {
                        //     let ct_lock = CURRENT_TASKS.lock();
                        //     ct_lock[cpu_id].set_current(None);
                        // }
                        // match poll_result { 
                        //     Poll::Ready(value) => {
                        //         info!("  task return {}.", value);
                        //     },
                        //     Poll::Pending => {
                        //         info!("  task not finished.");
                        //     },
                        // }
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