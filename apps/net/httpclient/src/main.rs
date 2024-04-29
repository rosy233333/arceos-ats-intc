#![cfg_attr(feature = "axstd", no_std)]
#![cfg_attr(feature = "axstd", no_main)]

#[macro_use]
#[cfg(feature = "axstd")]
extern crate axstd as std;

use core::time::Duration;
use std::io::{self, prelude::*};
use std::net::{TcpStream, ToSocketAddrs};
use std::time;

#[cfg(feature = "dns")]
const DEST: &str = "ident.me:80";
#[cfg(not(feature = "dns"))]
// const DEST: &str = "49.12.234.183:80";
const DEST: &str = "127.0.0.1:5555";

const REQUEST: &str = "\
GET / HTTP/1.1\r\n\
Host: ident.me\r\n\
Accept: */*\r\n\
\r\n";

fn client() -> io::Result<Duration> {
    // for addr in DEST.to_socket_addrs()? {
    //     println!("dest: {} ({})", DEST, addr);
    // }

    let start_time = time::Instant::now();
    let mut stream = TcpStream::connect(DEST)?;
    stream.write_all(REQUEST.as_bytes())?;
    let mut buf = [0; 2048];
    let n = stream.read(&mut buf)?;
    let end_time = time::Instant::now();
    // let response = core::str::from_utf8(&buf[..n]).unwrap();
    // println!("{}", response); // longer response need to handle tcp package problems.
    Ok(end_time - start_time)
}

#[cfg_attr(feature = "axstd", no_mangle)]
fn main() {
    println!("Hello, simple http client!");
    let mut results: Vec<Duration> = Vec::new();
    for i in 0 .. 16 {
        results.push(client().expect("test http client failed"));
    }
    for result in results {
        println!("{}", result.as_micros());
    }
}
