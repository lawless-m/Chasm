//! `/net/http` through the native namespace, against a local server.

#[path = "common/http_server.rs"]
mod http_server;

use wack_runtime::namespace::{Config, NativeHost};
use wack_runtime::Host;

fn host(net: bool) -> NativeHost {
    NativeHost::new(Config {
        net,
        ..Config::default()
    })
}

/// Read a handle to the end; a negative code is returned as `Err`.
fn read_all(h: &mut NativeHost, fd: i32) -> Result<String, i32> {
    let mut out = Vec::new();
    let mut buf = [0u8; 4];
    loop {
        match h.read(fd, &mut buf) {
            0 => return Ok(String::from_utf8(out).unwrap()),
            n if n < 0 => return Err(n),
            n => out.extend_from_slice(&buf[..n as usize]),
        }
    }
}

fn request(h: &mut NativeHost, url: &str, written: &[u8]) -> Result<String, i32> {
    let fd = h.open(url, 3);
    assert!(fd >= 0, "open {url}: {fd}");
    if !written.is_empty() {
        assert_eq!(h.write(fd, written), written.len() as i32);
    }
    let r = read_all(h, fd);
    assert_eq!(h.close(fd), 0);
    r
}

#[test]
fn get_post_and_errors() {
    let addr = http_server::start();
    let base = format!("/net/http/{addr}");
    let mut h = host(true);
    assert_eq!(
        request(&mut h, &format!("{base}/hello"), b""),
        Ok("hi\n".into())
    );
    assert_eq!(
        request(&mut h, &format!("{base}/echo"), b"X-Wack: 42\n\n"),
        Ok("42".into())
    );
    assert_eq!(
        request(&mut h, &format!("{base}/post"), b"X-Wack: 7\n\nhello"),
        Ok("got:hello:7".into())
    );
    assert_eq!(
        request(&mut h, &format!("{base}/post"), b"X-Wack: 7\nhello"),
        Err(-6)
    );
    assert_eq!(request(&mut h, &format!("{base}/missing"), b""), Err(-1));
    assert_eq!(request(&mut h, &format!("{base}/forbidden"), b""), Err(-2));
}

#[test]
fn write_after_read_is_refused() {
    let addr = http_server::start();
    let mut h = host(true);
    let fd = h.open(&format!("/net/http/{addr}/hello"), 3);
    let mut buf = [0u8; 16];
    assert_eq!(h.read(fd, &mut buf), 3);
    assert_eq!(h.write(fd, b"late"), -2);
}

#[test]
fn closed_port_and_disabled_net() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut h = host(true);
    assert_eq!(
        request(&mut h, &format!("/net/http/127.0.0.1:{port}/x"), b""),
        Err(-4)
    );
    assert_eq!(h.open("/net/gopher/x", 0), -3);
    let mut off = host(false);
    assert_eq!(off.open("/net/http/127.0.0.1/x", 0), -3);
}
