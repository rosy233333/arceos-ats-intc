#![no_std]
#![no_main]

mod parallel;
mod echo_server;
mod stats;

#[cfg(feature = "modified")]
use axstd::net::start_interrupt_or_poll;
use axstd::println;
use axstd::vec::Vec;
use crate::parallel::*;
use crate::echo_server::*;

use crate::stats::*;

#[no_mangle]
fn main() {
    println!("task test start.");

    #[cfg(not(feature = "output"))]
    const TASK_TEST_NUM: usize = 10;
    #[cfg(feature = "output")]
    const TASK_TEST_NUM: usize = 1;

    #[cfg(not(feature = "modified"))]
    {
        let mut used_time_us: Vec<u128> = Vec::new();
        for i in 0 .. TASK_TEST_NUM {
            #[cfg(not(feature = "output"))]
            used_time_us.push(test_task(840000, 240).as_micros());
            #[cfg(feature = "output")]
            used_time_us.push(test_task(84000, 10).as_micros());
        }
        println!("thread result stats:");
        println!("mean: {} μs, variance: {} μs", mean(&used_time_us).unwrap(), variance(&used_time_us).unwrap());
    }

    #[cfg(feature = "modified")]
    {
        let mut coroutine_used_time_us: Vec<u128> = Vec::new();
        for i in 0 .. TASK_TEST_NUM {
            #[cfg(not(feature = "output"))]
            coroutine_used_time_us.push(test_task_with_coroutine(840000, 240).as_micros());
            #[cfg(feature = "output")]
            coroutine_used_time_us.push(test_task_with_coroutine(84000, 10).as_micros());
        }
        println!("couroutine result stats:");
        println!("mean: {} μs, variance: {} μs", mean(&coroutine_used_time_us).unwrap(), variance(&coroutine_used_time_us).unwrap());
    }

    println!("net test start, running server...");

    #[cfg(feature = "modified")]
    unsafe {
        start_interrupt_or_poll();
    }

    run_server();
}