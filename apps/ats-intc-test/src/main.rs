#![no_std]
#![no_main]

use axstd::*;

#[no_mangle]
fn main() {
    // test_read_write();
    test_mmio_region();
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

pub fn test_mmio_region() {
    let executor_base: usize = 0xffff_ffc0_0f00_0000;
    let ps_0 = (executor_base + 0x00_0000) as *mut usize;
    let ih_0 = (executor_base + 0x00_0800) as *mut usize;
    let rs_0 = (executor_base + 0x00_0900) as *mut usize;
    let ps_1 = (executor_base + 0x00_1000) as *mut usize;
    let ih_1 = (executor_base + 0x00_1800) as *mut usize;
    let rs_1 = (executor_base + 0x00_1900) as *mut usize;
    let ps_4092 = (executor_base + 0xff_c000) as *mut usize;
    let ih_4092 = (executor_base + 0xff_c800) as *mut usize;
    let rs_4092 = (executor_base + 0xff_c900) as *mut usize;
    let eih = (executor_base + 0xff_d000) as *mut usize;
    let rs = (executor_base + 0xff_f128) as *mut usize;

    let k: usize = 15; // 0 <= k <= 15
    let process_offset = k * 0x1000; 

    let ps_control = (executor_base + process_offset + 0x00) as *mut usize;
    let ps_membuf = (executor_base + process_offset + 0x20) as *mut usize;
    let ps_dequeue = (executor_base + process_offset + 0x28) as *mut usize;
    let ps_enqueue_k = (executor_base + process_offset + 0x30 + k * 0x8) as *mut usize;

    let ih_control = (executor_base + process_offset + 0x0800 + 0x00) as *mut usize;
    let ih_membuf = (executor_base + process_offset + 0x0800 + 0x08) as *mut usize;
    let ih_message = (executor_base + process_offset + 0x0800 + 0x10) as *mut usize;
    let ih_bq_k = (executor_base + process_offset + 0x0800 + 0x18 + k * 0x8) as *mut usize;
    let ih_rs = (executor_base + process_offset + 0x0800 + 0x98) as *mut usize;

    let eih_control = (executor_base + 0xff_d000 + 0x00) as *mut usize;
    let eih_enqueue_k = (executor_base + 0xff_d000 + 0x80 + k * 0x8) as *mut usize;
    
    println!("k = {k}");
    println!("/////////////////////////");
    println!("////////READ TEST////////");
    println!("/////////////////////////");
    println!("--------Test Global MMIO--------");
    unsafe {
        assert_eq!(ps_0.read_volatile(), 0);
        assert_eq!(ih_0.read_volatile(), 0);
        assert_eq!(rs_0.read_volatile(), 0xffff_ffff_ffff_ffff);
        assert_eq!(ps_1.read_volatile(), 0);
        assert_eq!(ih_1.read_volatile(), 0);
        assert_eq!(rs_1.read_volatile(), 0xffff_ffff_ffff_ffff);
        assert_eq!(ps_4092.read_volatile(), 0);
        assert_eq!(ih_4092.read_volatile(), 0);
        assert_eq!(rs_4092.read_volatile(), 0xffff_ffff_ffff_ffff);
        assert_eq!(eih.read_volatile(), 0);
        assert_eq!(rs.read_volatile(), 0xffff_ffff_ffff_ffff);
    }
    println!("--------Test Priority Scheduler--------");
    unsafe {
        assert_eq!(ps_control.read_volatile(), 0);
        assert_eq!(ps_membuf.read_volatile(), 0);
        assert_eq!(ps_dequeue.read_volatile(), 0);
        assert_eq!(ps_enqueue_k.read_volatile(), 0);
    }
    println!("--------Test IPC Handler--------");
    unsafe {
        assert_eq!(ih_control.read_volatile(), 0);
        assert_eq!(ih_membuf.read_volatile(), 0);
        assert_eq!(ih_message.read_volatile(), 0);
        assert_eq!(ih_bq_k.read_volatile(), 0);
        assert_eq!(ih_rs.read_volatile(), 0xffff_ffff_ffff_ffff);
    }
    println!("--------Test Extern Interrupt Handler--------");
    unsafe {
        assert_eq!(eih_control.read_volatile(), 0);
        assert_eq!(eih_enqueue_k.read_volatile(), 0);
    }

    println!("//////////////////////////");
    println!("////////WRITE TEST////////");
    println!("//////////////////////////");
    println!("--------Test Global MMIO--------");
    unsafe {
        ps_0.write_volatile(0x0000_0000_0000_0000);
        ih_0.write_volatile(0x1111_1111_1111_1111);
        rs_0.write_volatile(0x2222_2222_2222_2222);
        ps_1.write_volatile(0x3333_3333_3333_3333);
        ih_1.write_volatile(0x4444_4444_4444_4444);
        rs_1.write_volatile(0x5555_5555_5555_5555);
        ps_4092.write_volatile(0x6666_6666_6666_6666);
        ih_4092.write_volatile(0x7777_7777_7777_7777);
        rs_4092.write_volatile(0x8888_8888_8888_8888);
        eih.write_volatile(0x9999_9999_9999_9999);
        rs.write_volatile(0xaaaa_aaaa_aaaa_aaaa);
    }
    println!("--------Test Priority Scheduler--------");
    unsafe {
        ps_control.write_volatile(0xbbbb_bbbb_bbbb_bbbb);
        ps_membuf.write_volatile(0xcccc_cccc_cccc_cccc);
        ps_dequeue.write_volatile(0xdddd_dddd_dddd_dddd);
        ps_enqueue_k.write_volatile(0xeeee_eeee_eeee_eeee);
    }
    println!("--------Test IPC Handler--------");
    unsafe {
        ih_control.write_volatile(0xffff_ffff_ffff_ffff);
        ih_membuf.write_volatile(0x0000_0000_0000_0000);
        ih_message.write_volatile(0x1111_1111_1111_1111);
        ih_bq_k.write_volatile(0x2222_2222_2222_2222);
        ih_rs.write_volatile(0x3333_3333_3333_3333);
    }
    println!("--------Test Extern Interrupt Handler--------");
    unsafe {
        eih_control.write_volatile(0x4444_4444_4444_4444);
        eih_enqueue_k.write_volatile(0x5555_5555_5555_5555);
    }
}