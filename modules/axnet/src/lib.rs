//! [ArceOS](https://github.com/rcore-os/arceos) network module.
//!
//! It provides unified networking primitives for TCP/UDP communication
//! using various underlying network stacks. Currently, only [smoltcp] is
//! supported.
//!
//! # Organization
//!
//! - [`TcpSocket`]: A TCP socket that provides POSIX-like APIs.
//! - [`UdpSocket`]: A UDP socket that provides POSIX-like APIs.
//! - [`dns_query`]: Function for DNS query.
//!
//! # Cargo Features
//!
//! - `smoltcp`: Use [smoltcp] as the underlying network stack. This is enabled
//!   by default.
//!
//! [smoltcp]: https://github.com/smoltcp-rs/smoltcp

#![no_std]
#![feature(ip_in_core)]
#![feature(new_uninit)]

#[macro_use]
extern crate log;
extern crate alloc;

cfg_if::cfg_if! {
    if #[cfg(feature = "smoltcp")] {
        mod smoltcp_impl;
        use smoltcp_impl as net_impl;
    }
}

use core::future::Future;
use core::future::Pending;
use core::sync::atomic::Ordering;
use core::task::Poll;
use core::time::Duration;

pub use self::net_impl::TcpSocket;
pub use self::net_impl::UdpSocket;
pub use self::net_impl::{bench_receive, bench_transmit};
pub use self::net_impl::dns_query;

use axdriver::{prelude::*, AxDeviceContainer};
use axtask::async_sleep;
use axtask::register_async_irq_handler;
use axtask::register_irq_handler;
use axtask::sleep;
use axtask::spawn;
use axtask::spawn_async;
use net_impl::poll_interfaces_return_delay;
use net_impl::poll_interfaces_return_delay_async;
pub use net_impl::poll_interfaces;
use net_impl::poll_interfaces_async;
#[cfg(feature = "block_queue")]
use net_impl::BLOCK_QUEUE;
#[cfg(any(feature = "interrupt", feature = "interrupt_async"))]
use net_impl::POLL_TASK;

const VIRTIO_NET_IRQ_NUM: usize = 8;

/// Initializes the network subsystem by NIC devices.
pub fn init_network(mut net_devs: AxDeviceContainer<AxNetDevice>) {
    info!("Initialize network subsystem...");

    let dev = net_devs.take_one().expect("No NIC device found!");
    info!("  use NIC 0: {:?}", dev.device_name());
    net_impl::init(dev);

    #[cfg(feature = "poll")]
    spawn(|| {
        loop {
            // let default_delay = core::time::Duration::from_secs(1);
            // let delay = poll_interfaces_return_delay();
            // match delay {
            //     Some(dur) => {
            //         // error!("delay is sone");
            //         sleep(dur.into());
            //     },
            //     None => {
            //         // error!("delay is none");
            //         sleep(default_delay);
            //     },
            // }
            // error!("poll thread running...");
            if poll_interfaces() {
                #[cfg(feature = "block_queue")]
                BLOCK_QUEUE.notify_all(true);
            }
            // axtask::yield_now();
        }
    });

    #[cfg(feature = "poll_async")]
    spawn_async(async {
        loop {
            // let default_delay = core::time::Duration::from_secs(1);
            // let delay = poll_interfaces_return_delay_async().await;
            // match delay {
            //     Some(dur) => {
            //         // error!("delay is sone");
            //         async_sleep(dur.into()).await;
            //     },
            //     None => {
            //         // error!("delay is none");
            //         async_sleep(default_delay).await;
            //     },
            // }
            if poll_interfaces_async().await {
                #[cfg(feature = "block_queue")]
                BLOCK_QUEUE.notify_all(true);
            }
            // axtask::yield_now_async().await;
        }
    });



    #[cfg(feature = "interrupt")]
    {
        spawn(|| {
            loop {
                // POLL_TASK.wait_timeout(Duration::from_millis(1));
                POLL_TASK.wait();
                // error!("poll task");
                while POLL_TASK.need_rewake.swap(false, Ordering::AcqRel) {
                    if poll_interfaces() {
                        #[cfg(feature = "block_queue")]
                        BLOCK_QUEUE.notify_all(true);
                    }
                }
            }
        }); // poll task
        register_irq_handler(VIRTIO_NET_IRQ_NUM, || {
            // error!("irq handler");
            POLL_TASK.notify();
            // error!("irq handler end");
        });
    }

    #[cfg(feature = "interrupt_async")]
    {
        spawn_async(async {
            loop {
                POLL_TASK.wait_async().await;
                while POLL_TASK.need_rewake.swap(false, Ordering::AcqRel) {
                    if poll_interfaces_async().await {
                        #[cfg(feature = "block_queue")]
                        BLOCK_QUEUE.notify_all(true);
                    }
                }
            }
            0
        });

        #[derive(Clone)]
        struct AsyncHandler { }
        impl Future for AsyncHandler {
            type Output = i32;
        
            fn poll(self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> Poll<Self::Output> {
                POLL_TASK.notify();
                Poll::Ready(0)
            }
        }

        register_async_irq_handler(VIRTIO_NET_IRQ_NUM, AsyncHandler { });  
    }
}
