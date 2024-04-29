#![cfg_attr(feature = "axstd", no_std)]
#![cfg_attr(feature = "axstd", no_main)]

#[macro_use]
#[cfg(feature = "axstd")]
extern crate axstd as std;

use std::io::{self, prelude::*};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::vec::Vec;

const LOCAL_IP: &str = "0.0.0.0";
const LOCAL_PORT: u16 = 5555;
const SEND_ADDR: &str = "127.0.0.1:5556";

fn reverse(buf: &[u8]) -> Vec<u8> {
    let mut lines = buf
        .split(|&b| b == b'\n')
        .map(Vec::from)
        .collect::<Vec<_>>();
    for line in lines.iter_mut() {
        line.reverse();
    }
    lines.join(&b'\n')
}

// fn echo_server(mut r_stream: TcpStream, mut w_stream: TcpStream) -> io::Result<()> {
fn echo_server(mut r_stream: TcpStream) -> io::Result<()> {
    const FRAME_SIZE : usize = 512;
    let mut buf = [0u8; FRAME_SIZE];
    loop {
        let n = r_stream.read(&mut buf)?;
        if n == 0 {
            r_stream.shutdown()?;
            return Ok(());
        }
        r_stream.write_all(&buf)?;
    }
}

fn accept_loop() -> io::Result<()> {
    let listener = TcpListener::bind((LOCAL_IP, LOCAL_PORT))?;
    println!("listen on: {}", listener.local_addr().unwrap());

    let mut i = 0;
    loop {
        match listener.accept() {
            Ok((r_stream, addr)) => {
                println!("new client {}: {}", i, addr);
                // let mut w_stream = TcpStream::connect(SEND_ADDR)?;
                thread::spawn(move || match echo_server(r_stream) {
                    Err(e) => println!("client connection error: {:?}", e),
                    Ok(()) => println!("client {} closed successfully", i),
                });
            }
            Err(e) => return Err(e),
        }
        i += 1;
    }
}

#[cfg_attr(feature = "axstd", no_mangle)]
fn main() {
    println!("Hello, echo server!");
    accept_loop().expect("test echo server failed");
}
