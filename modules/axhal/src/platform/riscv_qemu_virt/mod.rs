pub mod console {
    pub use crate::platform::riscv64_common::console::*;
}

pub(crate) mod mem {
    pub use crate::platform::riscv64_common::mem::*;
}

pub mod misc {
    pub use crate::platform::riscv64_common::misc::*;
}

pub mod time {
    pub use crate::platform::riscv64_common::time::*;
}

#[cfg(feature = "irq")]
pub mod irq {
    pub use crate::platform::riscv64_common::irq::*;
    pub use super::plic::*;
    pub fn init_ext_interrupt() {
        register_handler(8, || {
            debug!("dummy net interrupt handler");
        });
    }
}

#[cfg(feature = "smp")]
pub mod mp {
    pub use crate::platform::riscv64_common::mp::*;
}

pub use crate::platform::riscv64_common::platform_init;

#[cfg(feature = "smp")]
pub use crate::platform::riscv64_common::platform_init_secondary;

#[cfg(feature = "irq")]
pub mod plic;