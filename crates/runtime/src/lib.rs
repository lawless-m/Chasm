//! Chasm host runtime.
//!
//! The compiled module talks to its host through one doorbell import,
//! `chasm.ring_enter`, and a submission/completion ring in linear memory
//! (layout in `chasm_core::layout`). [`service_ring`] drains the ring against
//! any [`Host`]; [`namespace::NativeHost`] is the native namespace, and
//! [`native`] runs modules with wasmtime.

use chasm_core::layout as L;

pub mod namespace;
#[cfg(feature = "native")]
pub mod native;
pub mod net;
pub mod ninep;
#[cfg(feature = "native")]
pub mod repl;

/// The four I/O operations, over a namespace of paths.
pub trait Host {
    fn open(&mut self, path: &str, mode: i32) -> i32;
    fn read(&mut self, handle: i32, buf: &mut [u8]) -> i32;
    fn write(&mut self, handle: i32, buf: &[u8]) -> i32;
    fn close(&mut self, handle: i32) -> i32;
}

fn rd(mem: &[u8], addr: u32) -> u32 {
    let a = addr as usize;
    u32::from_le_bytes(mem[a..a + 4].try_into().unwrap())
}

fn wr(mem: &mut [u8], addr: u32, v: u32) {
    let a = addr as usize;
    mem[a..a + 4].copy_from_slice(&v.to_le_bytes());
}

fn range(mem: &[u8], addr: i32, len: i32) -> Option<std::ops::Range<usize>> {
    if addr < 0 || len < 0 {
        return None;
    }
    let (a, n) = (addr as usize, len as usize);
    (a.checked_add(n)? <= mem.len()).then_some(a..a + n)
}

/// Process every pending submission and post its completion.
pub fn service_ring(mem: &mut [u8], host: &mut dyn Host) {
    loop {
        let head = rd(mem, L::SQ_HEAD);
        let tail = rd(mem, L::SQ_TAIL);
        if head == tail {
            return;
        }
        let e = L::SQ_BASE + (head % L::RING_ENTRIES) * L::SQE_SIZE;
        let op = rd(mem, e + L::SQE_OP) as i32;
        let user = rd(mem, e + L::SQE_USER);
        let a0 = rd(mem, e + L::SQE_A0) as i32;
        let a1 = rd(mem, e + L::SQE_A1) as i32;
        let a2 = rd(mem, e + L::SQE_A2) as i32;
        let result = match op {
            L::OP_OPEN => match range(mem, a0, a1) {
                Some(r) => match std::str::from_utf8(&mem[r]) {
                    Ok(path) => {
                        let path = path.to_string();
                        host.open(&path, a2)
                    }
                    Err(_) => L::E_NOT_FOUND,
                },
                None => L::E_IO,
            },
            L::OP_READ => match range(mem, a1, a2) {
                Some(r) => host.read(a0, &mut mem[r]),
                None => L::E_IO,
            },
            L::OP_WRITE => match range(mem, a1, a2) {
                Some(r) => host.write(a0, &mem[r]),
                None => L::E_IO,
            },
            L::OP_CLOSE => host.close(a0),
            _ => L::E_NOT_SUPPORTED,
        };
        let ctail = rd(mem, L::CQ_TAIL);
        let c = L::CQ_BASE + (ctail % L::RING_ENTRIES) * L::CQE_SIZE;
        wr(mem, c + L::CQE_USER, user);
        wr(mem, c + L::CQE_RESULT, result as u32);
        wr(mem, L::CQ_TAIL, ctail.wrapping_add(1));
        wr(mem, L::SQ_HEAD, head.wrapping_add(1));
    }
}

/// Read the trap message cells a Chasm `trap` leaves behind, if any.
pub fn trap_info(mem: &[u8]) -> Option<(String, String)> {
    let get = |a: u32, l: u32| -> Option<String> {
        let r = range(mem, rd(mem, a) as i32, rd(mem, l) as i32)?;
        Some(String::from_utf8_lossy(&mem[r]).into_owned())
    };
    if rd(mem, L::TRAP_MSG_LEN) == 0 {
        return None;
    }
    Some((
        get(L::TRAP_MSG_ADDR, L::TRAP_MSG_LEN)?,
        get(L::TRAP_WORD_ADDR, L::TRAP_WORD_LEN).unwrap_or_default(),
    ))
}
