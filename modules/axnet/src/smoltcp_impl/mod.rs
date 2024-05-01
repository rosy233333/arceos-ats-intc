mod addr;
mod bench;
mod dns;
mod listen_table;
mod tcp;
mod udp;

use alloc::vec;
use axtask::WaitQueue;
use core::cell::RefCell;
use core::mem::ManuallyDrop;
use core::ops::DerefMut;
use core::sync;

use axdriver::prelude::*;
use axhal::time::{current_time_nanos, NANOS_PER_MICROS};
use axsync::{Mutex, MutexGuard};
use driver_net::{DevError, NetBufPtr};
use lazy_init::LazyInit;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::socket::{self, AnySocket};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr};

use self::listen_table::ListenTable;

pub use self::dns::dns_query;
pub use self::tcp::TcpSocket;
pub use self::udp::UdpSocket;

macro_rules! env_or_default {
    ($key:literal) => {
        match option_env!($key) {
            Some(val) => val,
            None => "",
        }
    };
}

const IP: &str = env_or_default!("AX_IP");
const GATEWAY: &str = env_or_default!("AX_GW");
const DNS_SEVER: &str = "8.8.8.8";
const IP_PREFIX: u8 = 24;

const STANDARD_MTU: usize = 1500;

const RANDOM_SEED: u64 = 0xA2CE_05A2_CE05_A2CE;

const TCP_RX_BUF_LEN: usize = 64 * 1024;
const TCP_TX_BUF_LEN: usize = 64 * 1024;
const UDP_RX_BUF_LEN: usize = 64 * 1024;
const UDP_TX_BUF_LEN: usize = 64 * 1024;
const LISTEN_QUEUE_SIZE: usize = 512;

static LISTEN_TABLE: LazyInit<ListenTable> = LazyInit::new();
static SOCKET_SET: LazyInit<SocketSetWrapper> = LazyInit::new();
static ETH0: LazyInit<InterfaceWrapper> = LazyInit::new();

#[cfg(feature = "block_queue")]
pub(crate) static BLOCK_QUEUE: WaitQueue = WaitQueue::new();
#[cfg(any(feature = "interrupt", feature = "interrupt_async"))]
pub(crate) static POLL_TASK: WaitQueue = WaitQueue::new();

struct SocketSetWrapper<'a>(Mutex<SocketSet<'a>>);

struct DeviceWrapper {
    inner: RefCell<AxNetDevice>, // use `RefCell` is enough since it's wrapped in `Mutex` in `InterfaceWrapper`.
}

struct InterfaceWrapper {
    name: &'static str,
    ether_addr: EthernetAddress,
    dev: Mutex<DeviceWrapper>,
    iface: Mutex<Interface>,
}

impl<'a> SocketSetWrapper<'a> {
    fn new() -> Self {
        Self(Mutex::new(SocketSet::new(vec![])))
    }

    pub fn new_tcp_socket() -> socket::tcp::Socket<'a> {
        let tcp_rx_buffer = socket::tcp::SocketBuffer::new(vec![0; TCP_RX_BUF_LEN]);
        let tcp_tx_buffer = socket::tcp::SocketBuffer::new(vec![0; TCP_TX_BUF_LEN]);
        socket::tcp::Socket::new(tcp_rx_buffer, tcp_tx_buffer)
    }

    pub fn new_udp_socket() -> socket::udp::Socket<'a> {
        let udp_rx_buffer = socket::udp::PacketBuffer::new(
            vec![socket::udp::PacketMetadata::EMPTY; 8],
            vec![0; UDP_RX_BUF_LEN],
        );
        let udp_tx_buffer = socket::udp::PacketBuffer::new(
            vec![socket::udp::PacketMetadata::EMPTY; 8],
            vec![0; UDP_TX_BUF_LEN],
        );
        socket::udp::Socket::new(udp_rx_buffer, udp_tx_buffer)
    }

    pub fn new_dns_socket() -> socket::dns::Socket<'a> {
        let server_addr = DNS_SEVER.parse().expect("invalid DNS server address");
        socket::dns::Socket::new(&[server_addr], vec![])
    }

    pub fn add<T: AnySocket<'a>>(&self, socket: T) -> SocketHandle {
        error!("try to get sockets lock");
        let handle = self.0.lock().add(socket);
        debug!("socket {}: created", handle);
        let res = handle;
        error!("release sockets lock");
        res
    }

    pub fn with_socket<T: AnySocket<'a>, R, F>(&self, handle: SocketHandle, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        error!("try to get sockets lock");
        let set = self.0.lock();
        let socket = set.get(handle);
        let res = f(socket);
        error!("release sockets lock");
        res
    }

    pub fn with_socket_mut<T: AnySocket<'a>, R, F>(&self, handle: SocketHandle, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        error!("try to get sockets lock");
        let mut set = self.0.lock();
        let socket = set.get_mut(handle);
        let res = f(socket);
        error!("release sockets lock");
        res
    }

    pub fn poll_interfaces(&self) -> bool {
        ETH0.poll(&self.0)
    }

    pub async fn poll_interfaces_async(&'static self) -> bool {
        ETH0.poll_async(&self.0).await
    }

    pub fn poll_interface_return_delay(&self) -> Option<smoltcp::time::Duration> {
        ETH0.poll(&self.0);
        ETH0.poll_delay(&self.0)
    }

    pub async fn poll_interface_return_delay_async(&'static self) -> Option<smoltcp::time::Duration> {
        ETH0.poll_async(&self.0).await;
        ETH0.poll_delay_async(&self.0).await
    }

    pub fn remove(&self, handle: SocketHandle) {
        error!("try to get sockets lock");
        self.0.lock().remove(handle);
        debug!("socket {}: destroyed", handle);
        error!("release sockets lock");
    }
}

impl InterfaceWrapper {
    fn new(name: &'static str, dev: AxNetDevice, ether_addr: EthernetAddress) -> Self {
        let mut config = Config::new(HardwareAddress::Ethernet(ether_addr));
        config.random_seed = RANDOM_SEED;

        let mut dev = DeviceWrapper::new(dev);
        let iface = Mutex::new(Interface::new(config, &mut dev, Self::current_time()));
        Self {
            name,
            ether_addr,
            dev: Mutex::new(dev),
            iface,
        }
    }

    fn current_time() -> Instant {
        Instant::from_micros_const((current_time_nanos() / NANOS_PER_MICROS) as i64)
    }

    pub fn name(&self) -> &str {
        self.name
    }

    pub fn ethernet_address(&self) -> EthernetAddress {
        self.ether_addr
    }

    pub fn setup_ip_addr(&self, ip: IpAddress, prefix_len: u8) {
        error!("try to get iface lock");
        let mut iface = self.iface.lock();
        iface.update_ip_addrs(|ip_addrs| {
            ip_addrs.push(IpCidr::new(ip, prefix_len)).unwrap();
        });
        error!("release iface lock");
    }

    pub fn setup_gateway(&self, gateway: IpAddress) {
        error!("try to get iface lock");
        let mut iface = self.iface.lock();
        match gateway {
            IpAddress::Ipv4(v4) => iface.routes_mut().add_default_ipv4_route(v4).unwrap(),
        };
        error!("release iface lock");
    }

    pub fn poll(&self, sockets: &Mutex<SocketSet>) -> bool {
        error!("try to get dev lock");
        let mut dev = self.dev.lock();
        error!("try to get iface lock");
        sync::atomic::compiler_fence(sync::atomic::Ordering::SeqCst);
        let mut iface = self.iface.lock();
        error!("try to get sockets lock");
        sync::atomic::compiler_fence(sync::atomic::Ordering::SeqCst);
        let mut sockets = sockets.lock();
        error!("get all locks");
        let timestamp = Self::current_time();
        let res = iface.poll(timestamp, dev.deref_mut(), &mut sockets);
        error!("release all locks");
        res
    }

    pub async fn poll_async(&'static self, sockets: &'static Mutex<SocketSet<'_>>) -> bool {
        let mut dev: ManuallyDrop<MutexGuard<'static, DeviceWrapper>> = ManuallyDrop::new(self.dev.lock_async().await);
        let mut iface: ManuallyDrop<MutexGuard<'static, Interface>> = ManuallyDrop::new(self.iface.lock_async().await);
        let mut sockets: ManuallyDrop<MutexGuard<'static, SocketSet<'_>>> = ManuallyDrop::new(sockets.lock_async().await);
        let timestamp = Self::current_time();
        let result = iface.poll(timestamp, dev.deref_mut().deref_mut(), &mut sockets);
        let _ = ManuallyDrop::into_inner(sockets);
        let _ = ManuallyDrop::into_inner(iface);
        let _ = ManuallyDrop::into_inner(dev);
        result
    }

    pub fn poll_delay(&self, sockets: &Mutex<SocketSet>) -> Option<smoltcp::time::Duration>{
        error!("try to get iface lock");
        let mut iface = self.iface.lock();
        sync::atomic::compiler_fence(sync::atomic::Ordering::SeqCst);
        error!("try to get sockets lock");
        let mut sockets = sockets.lock();
        error!("get all locks");
        let timestamp = Self::current_time();
        let res = iface.poll_delay(timestamp, &mut sockets);
        error!("release all locks");
        res
    }

    pub async fn poll_delay_async(&'static self, sockets: &'static Mutex<SocketSet<'_>>) -> Option<smoltcp::time::Duration>{
        let mut iface: ManuallyDrop<MutexGuard<'static, Interface>> = ManuallyDrop::new(self.iface.lock_async().await);
        let mut sockets: ManuallyDrop<MutexGuard<'static, SocketSet<'_>>> = ManuallyDrop::new(sockets.lock_async().await);
        let timestamp = Self::current_time();
        let res = iface.poll_delay(timestamp, &mut sockets);
        let _ = ManuallyDrop::into_inner(sockets);
        let _ = ManuallyDrop::into_inner(iface);
        res
    }
}

impl DeviceWrapper {
    fn new(inner: AxNetDevice) -> Self {
        Self {
            inner: RefCell::new(inner),
        }
    }
}

impl Device for DeviceWrapper {
    type RxToken<'a> = AxNetRxToken<'a> where Self: 'a;
    type TxToken<'a> = AxNetTxToken<'a> where Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let mut dev = self.inner.borrow_mut();
        if let Err(e) = dev.recycle_tx_buffers() {
            warn!("recycle_tx_buffers failed: {:?}", e);
            return None;
        }

        if !dev.can_transmit() {
            return None;
        }
        let rx_buf = match dev.receive() {
            Ok(buf) => buf,
            Err(err) => {
                if !matches!(err, DevError::Again) {
                    warn!("receive failed: {:?}", err);
                }
                return None;
            }
        };
        Some((AxNetRxToken(&self.inner, rx_buf), AxNetTxToken(&self.inner)))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        let mut dev = self.inner.borrow_mut();
        if let Err(e) = dev.recycle_tx_buffers() {
            warn!("recycle_tx_buffers failed: {:?}", e);
            return None;
        }
        if dev.can_transmit() {
            Some(AxNetTxToken(&self.inner))
        } else {
            None
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = 1514;
        caps.max_burst_size = None;
        caps.medium = Medium::Ethernet;
        caps
    }
}

struct AxNetRxToken<'a>(&'a RefCell<AxNetDevice>, NetBufPtr);
struct AxNetTxToken<'a>(&'a RefCell<AxNetDevice>);

impl<'a> RxToken for AxNetRxToken<'a> {
    fn preprocess(&self, sockets: &mut SocketSet<'_>) {
        snoop_tcp_packet(self.1.packet(), sockets).ok();
    }

    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut rx_buf = self.1;
        trace!(
            "RECV {} bytes: {:02X?}",
            rx_buf.packet_len(),
            rx_buf.packet()
        );
        let result = f(rx_buf.packet_mut());
        self.0.borrow_mut().recycle_rx_buffer(rx_buf).unwrap();
        result
    }
}

impl<'a> TxToken for AxNetTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut dev = self.0.borrow_mut();
        let mut tx_buf = dev.alloc_tx_buffer(len).unwrap();
        let ret = f(tx_buf.packet_mut());
        trace!("SEND {} bytes: {:02X?}", len, tx_buf.packet());
        dev.transmit(tx_buf).unwrap();
        ret
    }
}

fn snoop_tcp_packet(buf: &[u8], sockets: &mut SocketSet<'_>) -> Result<(), smoltcp::wire::Error> {
    use smoltcp::wire::{EthernetFrame, IpProtocol, Ipv4Packet, TcpPacket};

    let ether_frame = EthernetFrame::new_checked(buf)?;
    let ipv4_packet = Ipv4Packet::new_checked(ether_frame.payload())?;

    if ipv4_packet.next_header() == IpProtocol::Tcp {
        let tcp_packet = TcpPacket::new_checked(ipv4_packet.payload())?;
        let src_addr = (ipv4_packet.src_addr(), tcp_packet.src_port()).into();
        let dst_addr = (ipv4_packet.dst_addr(), tcp_packet.dst_port()).into();
        let is_first = tcp_packet.syn() && !tcp_packet.ack();
        if is_first {
            // create a socket for the first incoming TCP packet, as the later accept() returns.
            LISTEN_TABLE.incoming_tcp_packet(src_addr, dst_addr, sockets);
        }
    }
    Ok(())
}

/// Poll the network stack.
///
/// It may receive packets from the NIC and process them, and transmit queued
/// packets to the NIC.
pub fn poll_interfaces() -> bool {
    SOCKET_SET.poll_interfaces()
}

pub async fn poll_interfaces_async() -> bool {
    SOCKET_SET.poll_interfaces_async().await
}

pub fn poll_interfaces_return_delay() -> Option<smoltcp::time::Duration> {
    SOCKET_SET.poll_interface_return_delay()
}

pub async fn poll_interfaces_return_delay_async() -> Option<smoltcp::time::Duration> {
    SOCKET_SET.poll_interface_return_delay_async().await
}

/// Benchmark raw socket transmit bandwidth.
pub fn bench_transmit() {
    ETH0.dev.lock().bench_transmit_bandwidth();
}

/// Benchmark raw socket receive bandwidth.
pub fn bench_receive() {
    ETH0.dev.lock().bench_receive_bandwidth();
}

pub(crate) fn init(net_dev: AxNetDevice) {
    let ether_addr = EthernetAddress(net_dev.mac_address().0);
    let eth0 = InterfaceWrapper::new("eth0", net_dev, ether_addr);

    let ip = IP.parse().expect("invalid IP address");
    let gateway = GATEWAY.parse().expect("invalid gateway IP address");
    eth0.setup_ip_addr(ip, IP_PREFIX);
    eth0.setup_gateway(gateway);

    ETH0.init_by(eth0);
    SOCKET_SET.init_by(SocketSetWrapper::new());
    LISTEN_TABLE.init_by(ListenTable::new());

    info!("created net interface {:?}:", ETH0.name());
    info!("  ether:    {}", ETH0.ethernet_address());
    info!("  ip:       {}/{}", ip, IP_PREFIX);
    info!("  gateway:  {}", gateway);
}
