#![no_std]
#![no_main]

use axstd::println;
use axtask::*;

#[no_mangle]
fn main() {
    spawn(||{
        println!("1.1");
        yield_now();
        println!("1.2");
        yield_now();
        println!("1.3");
    });
    spawn(||{
        println!("2.1");
        yield_now();
        println!("2.2");
        yield_now();
        println!("2.3");
    });
    spawn(||{
        println!("3.1");
        yield_now();
        println!("3.2");
        yield_now();
        println!("3.3");
    });
    spawn_async(async || {
        atest("4.1").await;
        println!("4.2");
    });
    spawn_async(async || {
        atest("5.1").await;
        println!("5.2");
    });
    spawn_async(async || {
        atest("6.1").await;
        println!("6.2");
    });
}

async fn atest(str: &str) {
    println!(str);
}
