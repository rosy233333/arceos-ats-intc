#![no_std]
#![no_main]

use axstd::*;

#[no_mangle]
fn main() {
    test_read_write();
}

pub fn test_read_write() {
    let a = 0xffffffc00f00_0000 as *mut usize;
    unsafe { a.write_volatile(0x19990109) };
    unsafe { a.write_volatile(0x19990110) };
    let b = 0xffffffc00f00_0008 as *mut usize;
    println!("read res {:#X}", unsafe { b.read_volatile() });
    println!("read res {:#X}", unsafe { b.read_volatile() });
    println!("read res {:#X}", unsafe { b.read_volatile() });
}