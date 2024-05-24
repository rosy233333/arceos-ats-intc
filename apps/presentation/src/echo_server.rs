use axstd::io::{self, prelude::*};
use axstd::net::{TcpListener, TcpStream};
use axstd::{println, thread};
use axstd::vec::Vec;
use crate::parallel::sqrt;

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

fn echo_server(mut r_stream: TcpStream, client_id: usize) -> io::Result<()> {
    const FRAME_SIZE : usize = 2048;
    let mut buf = [0u8; FRAME_SIZE];
    loop {
        let mut result: u64 = 0;
        let mut read_pos: usize = 0;
        let mut compute_pos: usize = 0;
        while read_pos < FRAME_SIZE {
            {
                let n: usize = r_stream.read(&mut buf[read_pos ..])?;
                read_pos += n;
                if n == 0 {
                    thread::yield_now();
                    continue;
                }
                else {
                    #[cfg(feature = "output")]
                    println!("client {} read data, read size = {}, current read pos = {}", client_id, n, read_pos);
                }
            }
            {
                while compute_pos + 8 <= read_pos {
                    // #[cfg(feature = "output")]
                    // println!("client {} compute, current compute pos = {}", client_id, compute_pos);
                    let mut this_number: [u8; 8] = [0; 8];
                    this_number.copy_from_slice(&buf[compute_pos .. (compute_pos + 8)]);
                    result += sqrt(&u64::from_le_bytes(this_number));
                    compute_pos += 8;
                }
            }
        }
    
        r_stream.write_all(&result.to_le_bytes())?;
        r_stream.flush()?;
        
        #[cfg(feature = "output")]
        println!("client {} response", client_id);
    }
    // r_stream.shutdown()?;
}

fn accept_loop() -> io::Result<()> {
    let listener = TcpListener::bind((LOCAL_IP, LOCAL_PORT))?;
    println!("listen on: {}", listener.local_addr().unwrap());

    let mut i = 0;
    loop {
        match listener.accept() {
            Ok((r_stream, addr)) => {
                #[cfg(feature = "output")]
                println!("new client {}: {}", i, addr);
                thread::spawn(move || match echo_server(r_stream, i) {
                    Err(e) => {
                        println!("client {} connection error: {:?}", i, e)
                    },
                    Ok(()) => {
                        #[cfg(feature = "output")]
                        println!("client {} closed successfully", i)
                    },
                });
            }
            Err(e) => return Err(e),
        }
        i += 1;
    }
}

pub fn run_server() {
    accept_loop().expect("test echo server failed");
}
