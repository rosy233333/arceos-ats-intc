use alloc::{boxed::Box, collections::VecDeque};
use smoltcp::socket::AnySocket;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{compiler_fence, Ordering};

use axerrno::{ax_err, AxError, AxResult};
use axsync::Mutex;
use smoltcp::iface::{SocketHandle, SocketSet};
use smoltcp::socket::tcp::{self, State};
use smoltcp::wire::{IpAddress, IpEndpoint, IpListenEndpoint};

use super::{SocketSetWrapper, LISTEN_QUEUE_SIZE, SOCKET_SET};

const PORT_NUM: usize = 65536;

struct ListenTableEntry {
    listen_endpoint: IpListenEndpoint,
    syn_queue: VecDeque<SocketHandle>,
}

impl ListenTableEntry {
    pub fn new(listen_endpoint: IpListenEndpoint) -> Self {
        Self {
            listen_endpoint,
            syn_queue: VecDeque::with_capacity(LISTEN_QUEUE_SIZE),
        }
    }

    #[inline]
    fn can_accept(&self, dst: IpAddress) -> bool {
        match self.listen_endpoint.addr {
            Some(addr) => addr == dst,
            None => true,
        }
    }
}

impl Drop for ListenTableEntry {
    fn drop(&mut self) {
        for &handle in &self.syn_queue {
            SOCKET_SET.remove(handle);
        }
    }
}

pub struct ListenTable {
    tcp: Box<[Mutex<Option<Box<ListenTableEntry>>>]>,
}

impl ListenTable {
    pub fn new() -> Self {
        let tcp = unsafe {
            let mut buf = Box::new_uninit_slice(PORT_NUM);
            for i in 0..PORT_NUM {
                buf[i].write(Mutex::new(None));
            }
            buf.assume_init()
        };
        Self { tcp }
    }

    pub fn can_listen(&self, port: u16) -> bool {
        // error!("try to get listen table lock");
        let res = self.tcp[port as usize].lock().is_none();
        // error!("release listen table lock");
        res
    }

    pub fn listen(&self, listen_endpoint: IpListenEndpoint) -> AxResult {
        let port = listen_endpoint.port;
        assert_ne!(port, 0);
        // error!("try to get listen table lock");
        let mut entry = self.tcp[port as usize].lock();
        let res = if entry.is_none() {
            *entry = Some(Box::new(ListenTableEntry::new(listen_endpoint)));
            Ok(())
        } else {
            ax_err!(AddrInUse, "socket listen() failed")
        };
        // error!("release listen table lock");
        res
    }

    pub fn unlisten(&self, port: u16) {
        debug!("TCP socket unlisten on {}", port);
        // error!("try to get listen table lock");
        *self.tcp[port as usize].lock() = None;
        // error!("release listen table lock");
    }

    pub fn can_accept(&self, port: u16) -> AxResult<bool> {
        // error!("try to get sockets lock");
        let mut set = SOCKET_SET.0.lock();
        // compiler_fence(Ordering::SeqCst);
        // error!("try to get listen table lock");
        let res = if let Some(entry) = self.tcp[port as usize].lock().deref() {
            Ok(entry.syn_queue.iter().any(|&handle| is_connected_locked(&mut set, handle)))
        } else {
            ax_err!(InvalidInput, "socket accept() failed: not listen")
        };
        // error!("release all locks");
        res
    }

    pub fn accept(&self, port: u16) -> AxResult<(SocketHandle, (IpEndpoint, IpEndpoint))> {
        // error!("try to get sockets lock");
        let mut set = SOCKET_SET.0.lock();
        // compiler_fence(Ordering::SeqCst);
        // error!("try to get listen table lock");
        let res = if let Some(entry) = self.tcp[port as usize].lock().deref_mut() {
            let syn_queue = &mut entry.syn_queue;
            // let (idx, addr_tuple) = syn_queue
            //     .iter()
            //     .enumerate()
            //     .find_map(|(idx, &handle)| {
            //         is_connected_locked(&mut set, handle).then(|| (idx, get_addr_tuple_locked(&mut set, handle)))
            //     })
            //     .ok_or(AxError::WouldBlock)?; // wait for connection
            let opt = syn_queue
                .iter()
                .enumerate()
                .find_map(|(idx, &handle)| {
                    is_connected_locked(&mut set, handle).then(|| (idx, get_addr_tuple_locked(&mut set, handle)))
                });
            if let Some((idx, addr_tuple)) = opt {
                if idx > 0 {
                    warn!(
                        "slow SYN queue enumeration: index = {}, len = {}!",
                        idx,
                        syn_queue.len()
                    );
                }
                let handle = syn_queue.swap_remove_front(idx).unwrap();
                Ok((handle, addr_tuple))
            }
            else {
                // error!("release all locks");
                Err(AxError::WouldBlock)
            }
        } else {
            ax_err!(InvalidInput, "socket accept() failed: not listen")
        };
        // error!("release all locks");
        res
    }

    pub fn incoming_tcp_packet(
        &self,
        src: IpEndpoint,
        dst: IpEndpoint,
        sockets: &mut SocketSet<'_>,
    ) {
        // error!("try to get listen table lock");
        if let Some(entry) = self.tcp[dst.port as usize].lock().deref_mut() {
            if !entry.can_accept(dst.addr) {
                // not listening on this address
                return;
            }
            if entry.syn_queue.len() >= LISTEN_QUEUE_SIZE {
                // SYN queue is full, drop the packet
                warn!("SYN queue overflow!");
                return;
            }
            let mut socket = SocketSetWrapper::new_tcp_socket();
            if socket.listen(entry.listen_endpoint).is_ok() {
                let handle = sockets.add(socket);
                debug!(
                    "TCP socket {}: prepare for connection {} -> {}",
                    handle, src, entry.listen_endpoint
                );
                entry.syn_queue.push_back(handle);
            }
        }
        // error!("release listen table lock");
    }
}

fn with_socket_locked<'a, T: AnySocket<'a>, R, F>(set: &mut SocketSet<'a>, handle: SocketHandle, f: F) -> R
where F: FnOnce(&T) -> R, {
    let socket = set.get(handle);
    f(socket)
}

fn is_connected(handle: SocketHandle) -> bool {
    SOCKET_SET.with_socket::<tcp::Socket, _, _>(handle, |socket| {
        !matches!(socket.state(), State::Listen | State::SynReceived)
    })
}

fn is_connected_locked(set: &mut SocketSet, handle: SocketHandle) -> bool {
    with_socket_locked::<tcp::Socket, _, _>(set, handle, |socket| {
        !matches!(socket.state(), State::Listen | State::SynReceived)
    })
}

fn get_addr_tuple(handle: SocketHandle) -> (IpEndpoint, IpEndpoint) {
    SOCKET_SET.with_socket::<tcp::Socket, _, _>(handle, |socket| {
        (
            socket.local_endpoint().unwrap(),
            socket.remote_endpoint().unwrap(),
        )
    })
}

fn get_addr_tuple_locked(set: &mut SocketSet, handle: SocketHandle) -> (IpEndpoint, IpEndpoint) {
    with_socket_locked::<tcp::Socket, _, _>(set, handle, |socket| {
        (
            socket.local_endpoint().unwrap(),
            socket.remote_endpoint().unwrap(),
        )
    })
}