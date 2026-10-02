//! A tiny HTTP/1.1 server for offline tests: one request per connection.
//!
//! GET /hello → `hi\n`; GET /echo → the `X-Chasm` header or `none`;
//! POST /post → `got:<body>:<X-Chasm or none>`; /missing → 404;
//! /forbidden → 403.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};

pub fn start() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || serve(stream));
        }
    });
    addr
}

fn serve(stream: TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut length = 0;
    let mut chasm = "none".to_string();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).unwrap_or(0) == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((name, value)) = h.split_once(':') {
            let value = value.trim().to_string();
            match name.to_ascii_lowercase().as_str() {
                "content-length" => length = value.parse().unwrap_or(0),
                "x-chasm" => chasm = value,
                _ => {}
            }
        }
    }
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    let body = String::from_utf8_lossy(&body).into_owned();
    let (status, text) = match (method.as_str(), path.as_str()) {
        ("GET", "/hello") => ("200 OK", "hi\n".to_string()),
        ("GET", "/echo") => ("200 OK", chasm),
        ("POST", "/post") => ("200 OK", format!("got:{body}:{chasm}")),
        (_, "/forbidden") => ("403 Forbidden", String::new()),
        _ => ("404 Not Found", String::new()),
    };
    let mut stream = stream;
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
    let _ = stream.flush();
}
