#![no_std]
#![no_main]

mod executor;

extern crate alloc;
use alloc::boxed::Box;
use axstd::*;
use executor::Executor;
use ats_intc::Task;
use ats_intc::TaskType;

#[no_mangle]
fn main() {
    let executor = Executor::new(0);
    executor.spawn(Task::new(Box::pin(task_test(3)), 3, TaskType::Other));
    executor.spawn(Task::new(Box::pin(task_test(0)), 0, TaskType::Other));
    executor.spawn(Task::new(Box::pin(task_test(2)), 2, TaskType::Other));
    executor.spawn(Task::new(Box::pin(task_test(1)), 1, TaskType::Other));
    executor.run();
}

async fn task_test(i: usize) -> i32 {
    println!("async task priority {i}");
    0
}
