use axstd::io::{self, prelude::*};
use axstd::net::{TcpListener, TcpStream};
use axstd::{println, thread};
use axstd::vec::Vec;

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

fn echo_server(mut r_stream: TcpStream) -> io::Result<()> {
    const FRAME_SIZE : usize = 1024;
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
                #[cfg(feature = "output")]
                println!("new client {}: {}", i, addr);
                thread::spawn(move || match echo_server(r_stream) {
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
