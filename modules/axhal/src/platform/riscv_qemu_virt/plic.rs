use rv_plic::Priority;
use rv_plic::PLIC;
use crate::cpu::this_cpu_id;
use axconfig::{NET_IRQ, PLIC_PADDR, PHYS_VIRT_OFFSET};

pub const PLIC_BASE: usize = PLIC_PADDR + PHYS_VIRT_OFFSET;
pub const PLIC_PRIORITY_BIT: usize = 2;
pub type Plic = PLIC<PLIC_BASE, PLIC_PRIORITY_BIT>;


pub fn get_context(hartid: usize, mode: char) -> usize {
    const MODE_PER_HART: usize = 2;
    hartid * MODE_PER_HART
        + match mode {
            'M' => 0,
            'S' => 1,
            _ => panic!("Wrong Mode"),
        }
}

pub fn enable(scause: usize, _enabled: bool) {
    trace!("set enable {}", scause);
    match scause {
        NET_IRQ => {
            let hart_id = this_cpu_id();
            Plic::set_priority(scause as _, Priority::lowest());
            let context = get_context(hart_id, 'S');
            Plic::clear_enable(context, 0);
            Plic::set_threshold(context, Priority::any());
            Plic::set_threshold(get_context(hart_id, 'M'), Priority::never());
            Plic::enable(context, scause as _);
            Plic::claim(context);
            Plic::complete(context, scause as _);
        },
        _ => panic!("Not supported externel interrupt!"),
    };
}

pub fn get_irq() -> Option<u16> {
    let hart_id = this_cpu_id();
    let context = get_context(hart_id, 'S');
    Plic::claim(context)
}
