//! `/mnt/<name>` served by a 9p server, through the native namespace.

#[path = "common/ninep_server.rs"]
mod server;

use wack_runtime::namespace::{Config, Mount, NativeHost};
use wack_runtime::Host;

fn host(addr: String) -> NativeHost {
    NativeHost::new(Config {
        mounts: [("p".to_string(), Mount::NineP(addr))].into(),
        ..Config::default()
    })
}

fn read_all(h: &mut NativeHost, path: &str) -> Result<Vec<u8>, i32> {
    let fd = h.open(path, 0);
    if fd < 0 {
        return Err(fd);
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        match h.read(fd, &mut buf) {
            0 => break,
            n if n < 0 => return Err(n),
            n => out.extend_from_slice(&buf[..n as usize]),
        }
    }
    h.close(fd);
    Ok(out)
}

/// (name, size, is-dir) from directory records.
fn records(mut b: &[u8]) -> Vec<(String, u64, bool)> {
    let mut out = Vec::new();
    while !b.is_empty() {
        let n = u32::from_le_bytes(b[..4].try_into().unwrap()) as usize;
        let name = String::from_utf8(b[4..4 + n].to_vec()).unwrap();
        let size = u64::from_le_bytes(b[4 + n..12 + n].try_into().unwrap());
        out.push((name, size, b[12 + n] != 0));
        b = &b[13 + n..];
    }
    out
}

#[test]
fn read_a_mounted_tree() {
    let mut h = host(server::start().to_string());
    let dir = read_all(&mut h, "/mnt/p").unwrap();
    assert_eq!(
        records(&dir),
        [
            ("hello.txt".to_string(), 6, false),
            ("sub".to_string(), 0, true)
        ]
    );
    let fd = h.open("/mnt/p/hello.txt", 0);
    let mut buf = [0u8; 3];
    assert_eq!(h.read(fd, &mut buf), 3);
    assert_eq!(&buf, b"hel");
    assert_eq!(h.read(fd, &mut buf), 3);
    assert_eq!(&buf, b"lo\n");
    assert_eq!(h.read(fd, &mut buf), 0);
    assert_eq!(h.close(fd), 0);
    assert_eq!(read_all(&mut h, "/mnt/p/sub/a.txt").unwrap(), b"a");
    assert_eq!(read_all(&mut h, "/mnt/p/missing"), Err(-1));
    assert_eq!(h.open("/mnt/p/../x", 0), -2);
}

#[test]
fn write_create_and_append() {
    let mut h = host(server::start().to_string());
    let fd = h.open("/mnt/p/new.txt", 1);
    assert!(fd >= 0, "{fd}");
    assert_eq!(h.write(fd, b"one"), 3);
    h.close(fd);
    assert_eq!(read_all(&mut h, "/mnt/p/new.txt").unwrap(), b"one");
    let fd = h.open("/mnt/p/new.txt", 2);
    assert_eq!(h.write(fd, b"two"), 3);
    h.close(fd);
    assert_eq!(read_all(&mut h, "/mnt/p/new.txt").unwrap(), b"onetwo");
}

#[test]
fn unreachable_server_is_an_io_error() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut h = host(format!("127.0.0.1:{port}"));
    assert_eq!(h.open("/mnt/p/hello.txt", 0), -4);
}
