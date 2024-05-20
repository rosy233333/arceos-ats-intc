use rand::{rngs::SmallRng, RngCore, SeedableRng};
use core::time::{self, Duration};
use axstd::thread::{self, sleep, yield_now};
use axstd::{sync::Arc, vec::Vec};
use axstd::time::Instant;
use axstd::println;

const NUM_DATA: usize = 840000;
const NUM_TASKS: usize = 80;

fn sqrt(n: &u64) -> u64 {
    let mut x = *n;
    loop {
        if x * x <= *n && (x + 1) * (x + 1) > *n {
            return x;
        }
        x = (x + *n / x) / 2;
    }
}

pub fn test_task() -> Duration {
    let mut rng = SmallRng::seed_from_u64(0xdead_beef);
    let vec = Arc::new(
        (0..NUM_DATA)
            .map(|_| rng.next_u32() as u64)
            .collect::<Vec<_>>(),
    );
    let expect: u64 = vec.iter().map(sqrt).sum();
    #[cfg(feature = "output")]
    println!("expect sum = {}", expect);

    let mut tasks = Vec::with_capacity(NUM_TASKS);
    let start_time: Instant = Instant::now();
    for i in 0..NUM_TASKS {
        let vec = vec.clone();
        tasks.push(thread::spawn(move || {
            let left = i * (NUM_DATA / NUM_TASKS);
            let right = (left + (NUM_DATA / NUM_TASKS)).min(NUM_DATA);
            #[cfg(feature = "output")]
            println!(
                "part {}: {:?} [{}, {})",
                i,
                thread::current().id(),
                left,
                right
            );

            let partial_sum: u64 = vec[left..right].iter().map(sqrt).sum();

            #[cfg(feature = "output")]
            println!("part {}: {:?} finished", i, thread::current().id());
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
pub fn test_task_with_coroutine() -> Duration {

    let mut rng = SmallRng::seed_from_u64(0xdead_beef);
    let vec = Arc::new(
        (0..NUM_DATA)
            .map(|_| rng.next_u32() as u64)
            .collect::<Vec<_>>(),
    );
    let expect: u64 = vec.iter().map(sqrt).sum();
    #[cfg(feature = "output")]
    println!("expect sum = {}", expect);

    let mut tasks = Vec::with_capacity(NUM_TASKS);
    let start_time: Instant = Instant::now();
    for i in 0..NUM_TASKS {
        let vec = vec.clone();
        tasks.push(thread::spawn_async(async move {
            let left = i * (NUM_DATA / NUM_TASKS);
            let right = (left + (NUM_DATA / NUM_TASKS)).min(NUM_DATA);
            #[cfg(feature = "output")]
            println!(
                "part {}: {:?} [{}, {})",
                i,
                thread::current().id(),
                left,
                right
            );

            let partial_sum: u64 = vec[left..right].iter().map(sqrt).sum();

            #[cfg(feature = "output")]
            println!("part {}: {:?} finished", i, thread::current().id());
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