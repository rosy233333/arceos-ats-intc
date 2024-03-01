use core::hint::spin_loop;
use core::task::Poll;
use core::sync::atomic::Ordering;

use alloc::sync::Arc;
use axstd::println;

use ats_intc::{Task, TaskRef};

const HW_ADDR_BASE: usize = 0xffff_ffc0_0f00_0000;

pub struct Executor {
    process_id: u32,
}

impl Executor {
    pub fn new(process_id: u32) -> Self {
        Executor{process_id}
    }

    pub fn spawn(&self, task_ref: TaskRef) {
        let priority = unsafe { (*task_ref.as_ptr()).priority.load(Ordering::Acquire) };

        unsafe {
            // 将task_ref写入调度器硬件的对应地址，地址的计算参考https://ats-intc.github.io/docs/ats-intc/01register-space/
            let hw_addr: usize = HW_ADDR_BASE + (self.process_id as usize) * 0x1000 + 0x30 + (priority as usize) * 0x8;
            (hw_addr as *mut usize).write_volatile(task_ref.as_ptr() as usize);
        }
    }

    fn fetch_task(&self) -> Option<TaskRef> {
        let task_ptr: usize = unsafe {
            // 从调度器硬件的对应地址获得task指针，地址的计算参考https://ats-intc.github.io/docs/ats-intc/01register-space/
            let hw_addr: usize = HW_ADDR_BASE + (self.process_id as usize) * 0x1000 + 0x28;
            (hw_addr as *const usize).read_volatile()
        };
        if task_ptr != 0 {
            Some(unsafe { TaskRef::from_ptr(task_ptr as *const Task) })
        }
        else {
            None
        }
    }

    pub fn run(&self) -> ! {
        loop {
            match self.fetch_task() {
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