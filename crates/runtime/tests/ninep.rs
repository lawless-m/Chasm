//! The 9P2000 client against the in-test server.

#[path = "common/ninep_server.rs"]
mod server;

use wack_runtime::ninep::{Client, Error, OREAD, OWRITE};

#[test]
fn read_files_and_directories() {
    let addr = server::start();
    let mut c = Client::connect(&addr.to_string()).unwrap();
    let f = c.walk("hello.txt").unwrap();
    c.open(f, OREAD).unwrap();
    assert_eq!(c.read(f, 0, 100).unwrap(), b"hello\n");
    assert_eq!(c.stat(f).unwrap().length, 6);
    c.clunk(f).unwrap();
    let d = c.walk("").unwrap();
    assert!(c.open(d, OREAD).unwrap().is_dir());
    let entries = c.read_dir(d).unwrap();
    let names: Vec<(&str, u64, bool)> = entries
        .iter()
        .map(|s| (s.name.as_str(), s.length, s.is_dir))
        .collect();
    assert_eq!(names, [("hello.txt", 6, false), ("sub", 0, true)]);
    let a = c.walk("sub/a.txt").unwrap();
    c.open(a, OREAD).unwrap();
    assert_eq!(c.read(a, 0, 10).unwrap(), b"a");
    assert_eq!(c.walk("missing"), Err(Error::NotFound));
    assert_eq!(c.walk("sub/missing"), Err(Error::NotFound));
}

#[test]
fn create_write_and_read_back() {
    let addr = server::start();
    let mut c = Client::connect(&addr.to_string()).unwrap();
    let dir = c.walk("").unwrap();
    c.create(dir, "new.txt", 0o644, OWRITE).unwrap();
    assert_eq!(c.write(dir, 0, b"xyz").unwrap(), 3);
    c.clunk(dir).unwrap();
    let f = c.walk("new.txt").unwrap();
    c.open(f, OREAD).unwrap();
    assert_eq!(c.read(f, 0, 10).unwrap(), b"xyz");
}

#[test]
fn nothing_listening_is_an_io_error() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    assert!(Client::connect(&format!("127.0.0.1:{port}")).is_err());
}
