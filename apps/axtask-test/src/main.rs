#![no_std]
#![no_main]

extern crate alloc;

use core::future::Future;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;
use core::task::Poll;

use alloc::string::String;
use alloc::string::ToString;
use axstd::println;
use axstd::thread;

#[no_mangle]
fn main() {
    println!("0.1");
    thread::spawn(|| {
        println!("1.1");
        thread::yield_now();
        println!("1.2");
        thread::yield_now();
        println!("1.3");
    });
    thread::spawn(|| {
        println!("2.1");
        thread::yield_now();
        println!("2.2");
        thread::yield_now();
        println!("2.3");
    });
    thread::spawn(|| {
        println!("3.1");
        thread::yield_now();
        println!("3.2");
        thread::yield_now();
        println!("3.3");
    });
    thread::spawn_async(async {
        println!("4.1");
        AsyncTest::new("4.2").await;
        AsyncTest::new("4.3").await;
        0
    });
    thread::spawn_async(async {
        println!("5.1");
        AsyncTest::new("5.2").await;
        AsyncTest::new("5.3").await;
        0
    });
    thread::spawn_async(async {
        println!("6.1");
        AsyncTest::new("6.2").await;
        AsyncTest::new("6.3").await;
        0
    });
    println!("0.2");
}

struct AsyncTest {
    a: AtomicBool,
    string: String,
}

/// 第一次调用时，返回Pending
/// 第二次调用时，打印字符串并返回Ready
impl AsyncTest {
    fn new(str: &str) -> Self {
        Self {
            a: AtomicBool::new(false),
            string: String::from(str),
        }
    }
}

impl Future for AsyncTest {
    type Output = ();

    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> core::task::Poll<Self::Output> {
        if self.a.load(Ordering::Relaxed) {
            println!("{}", self.string);
            self.a.store(false, Ordering::Relaxed);
            Poll::Ready(())
        }
        else {
            self.a.store(true, Ordering::Relaxed);
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

// #[no_mangle]
// fn main() {
//     println!("0.1");
// }
