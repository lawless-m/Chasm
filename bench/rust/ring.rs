// The same work as bench/ring.chasm without the ring: one system call per
// one-byte read or write, and SystemTime::now. Run:
//   rustc -C opt-level=3 -o tmp/ring-rs bench/rust/ring.rs
//   tmp/ring-rs tmp/ring
use std::fs::File;
use std::io::{Read, Write};
use std::time::{Instant, SystemTime};

fn main() {
    let dir = std::env::args().nth(1).expect("usage: ring-rs DIR");
    let n = 200_000u32;
    let data = vec![0u8; n as usize];
    File::create(format!("{dir}/data")).unwrap().write_all(&data).unwrap();

    let mut r = File::open(format!("{dir}/data")).unwrap();
    let mut b = [0u8; 1];
    let t = Instant::now();
    for _ in 0..n {
        r.read(&mut b).unwrap();
    }
    let reads = t.elapsed().as_nanos() / n as u128;

    let mut w = File::create(format!("{dir}/out")).unwrap();
    let t = Instant::now();
    for _ in 0..n {
        w.write(&b).unwrap();
    }
    let writes = t.elapsed().as_nanos() / n as u128;

    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(SystemTime::now());
    }
    let clocks = t.elapsed().as_nanos() / n as u128;

    println!("read-1B {reads}");
    println!("write-1B {writes}");
    println!("now {clocks}");
}
