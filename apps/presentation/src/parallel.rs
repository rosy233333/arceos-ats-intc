use rand::{rngs::SmallRng, RngCore, SeedableRng};
use core::time::{self, Duration};
use axstd::thread::{self, sleep, yield_now};
use axstd::{sync::Arc, vec::Vec};
use axstd::time::Instant;
use axstd::println;

// const NUM_DATA: usize = 840000;
// const NUM_TASKS: usize = 240;

pub fn sqrt(n: &u64) -> u64 {
    let mut x = *n;
    loop {
        if x * x <= *n && (x + 1) * (x + 1) > *n {
            return x;
        }
        x = (x + *n / x) / 2;
    }
}

pub fn test_task(num_data: usize, num_tasks: usize) -> Duration {
    let mut rng = SmallRng::seed_from_u64(0xdead_beef);
    let vec = Arc::new(
        (0..num_data)
            .map(|_| rng.next_u32() as u64)
            .collect::<Vec<_>>(),
    );
    let expect: u64 = vec.iter().map(sqrt).sum();
    #[cfg(feature = "output")]
    println!("expect sum = {}", expect);

    let mut tasks = Vec::with_capacity(num_tasks);
    let start_time: Instant = Instant::now();
    for i in 0 .. num_tasks {
        let vec = vec.clone();
        tasks.push(thread::spawn(move || {
            let left = i * (num_data / num_tasks);
            let right = (left + (num_data / num_tasks)).min(num_data);
            #[cfg(feature = "output")]
            println!(
                "part {}: ThreadId({}) [{}, {})",
                i,
                thread::current().id().as_u64(),
                left,
                right
            );

            let partial_sum: u64 = vec[left..right].iter().map(sqrt).sum();

            #[cfg(feature = "output")]
            println!("part {}: ThreadId({}) finished", i, thread::current().id().as_u64());
            partial_sum
        }));
    }

    let actual: u64 = tasks.into_iter().map(|t| t.join().unwrap()).sum();
    let end_time: Instant = Instant::now();
    let used_time = end_time.duration_since(start_time);
    #[cfg(feature = "output")]
    println!("actual sum = {}", actual);

    assert_eq!(expect, actual);

    println!("test ok, used time = {:?}", used_time);
    used_time
}

#[cfg(feature = "modified")]
pub fn test_task_with_coroutine(num_data: usize, num_tasks: usize) -> Duration {

    let mut rng = SmallRng::seed_from_u64(0xdead_beef);
    let vec = Arc::new(
        (0..num_data)
            .map(|_| rng.next_u32() as u64)
            .collect::<Vec<_>>(),
    );
    let expect: u64 = vec.iter().map(sqrt).sum();
    #[cfg(feature = "output")]
    println!("expect sum = {}", expect);

    let mut tasks = Vec::with_capacity(num_tasks);
    let start_time: Instant = Instant::now();
    for i in 0..num_tasks {
        let vec = vec.clone();
        tasks.push(thread::spawn_async(async move {
            let left = i * (num_data / num_tasks);
            let right = (left + (num_data / num_tasks)).min(num_data);
            #[cfg(feature = "output")]
            println!(
                "part {}: CoroutineId({}) [{}, {})",
                i,
                thread::current().id().as_u64(),
                left,
                right
            );

            let partial_sum: u64 = vec[left..right].iter().map(sqrt).sum();

            #[cfg(feature = "output")]
            println!("part {}: CoroutineId({}) finished", i, thread::current().id().as_u64());
            partial_sum
        }));
    }

    let actual: u64 = tasks.into_iter().map(|t| t.join().unwrap()).sum();
    let end_time: Instant = Instant::now();
    let used_time = end_time.duration_since(start_time);
    #[cfg(feature = "output")]
    println!("actual sum = {}", actual);

    assert_eq!(expect, actual);

    println!("test ok, used time = {:?}", used_time);
    used_time
}