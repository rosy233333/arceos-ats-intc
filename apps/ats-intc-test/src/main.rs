#![no_std]
#![no_main]

use core::{alloc::Layout, task::{Context, Poll}, sync::atomic::Ordering, pin::Pin, future::Future};
use axstd::{println, os::arceos::api::mem::ax_alloc};


use ats_intc::*;
use axstd::boxed::Box;

#[no_mangle]
fn main() {
    let a = ax_alloc(Layout::from_size_align(0x1000, 0x1000).unwrap());
    let ats_driver = AtsDriver::new(a.unwrap().as_ptr() as usize);
    ats_driver.reset();
    ats_driver.load_handler(8, Task::new(Box::new(net_handler()), 0, TaskType::Other));
    loop {
        if let Some(task_ref) = ats_driver.ftask() {
            unsafe {
                let waker = from_task(task_ref);
                let mut cx = Context::from_waker(&waker);
                let task = Task::from_ref(task_ref);
                task.state.store(TaskState::Running as _, Ordering::Relaxed);
                let fut = &mut *task.fut.as_ptr();
                let mut future = Pin::new_unchecked(fut.as_mut());
                let _ = future.as_mut().poll(&mut cx);
                ats_driver.load_handler(8, task.as_ref());
            }
        }
    }
    
    
}

#[no_mangle]
fn cpu_num() -> usize {
    4
}

async fn net_handler() -> i32 {
    let mut helper = Box::new(Helper::default());
    loop {
        println!("Hello, world!");
        helper.as_mut().await;
    }
}

#[derive(Default)]
pub struct Helper(usize);



impl Future for Helper {
    type Output = i32;
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.0 += 1;
        if self.0 & 1 == 0 {
            Poll::Ready(0)
        } else {
            Poll::Pending
        }
    }
}





