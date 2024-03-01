use core::hint::spin_loop;
use core::task::Poll;
use core::sync::atomic::Ordering;

use alloc::sync::Arc;
use axstd::println;

use ats_intc::{Task, TaskRef, AtsDriver};

const HW_ADDR_BASE: usize = 0xffff_ffc0_0f00_0000;

pub struct Executor {
    hw_executor: AtsDriver,
    process_id: usize,
}

impl Executor {
    pub fn new(process_id: usize) -> Self {
        Executor{hw_executor: AtsDriver::new(HW_ADDR_BASE), process_id}
    }

    pub fn spawn(&self, task_ref: TaskRef) {
        let priority = unsafe { (*task_ref.as_ptr()).priority.load(Ordering::Acquire) };
        self.hw_executor.stask(task_ref, self.process_id, priority as usize);
    }

    pub fn run(&self) -> ! {
        loop {
            match self.hw_executor.ftask(self.process_id) {
                Some(task_ref) => {
                    let task_arc = Task::from_ref(task_ref);
                    let result: Poll<i32> = task_arc.poll_inner();
                    println!("executor get result {:?}", result);
                },
                None => {
                    spin_loop();
                }
            }
        }
    }
}