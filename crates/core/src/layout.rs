//! Linear-memory layout shared by the compiler, the host and (later) the REPL.
//!
//! These are v1 defaults, not promises. They live here and nowhere else.

pub const PAGE: u32 = 64 * 1024;

/// 0 to 64 KiB is reserved. Address 0 is never valid. A few runtime cells
/// live in this region, well clear of 0.
pub const RESERVED_END: u32 = 0x1_0000;

/// Runtime cells (all `u32`, little-endian).
pub const TRAP_MSG_ADDR: u32 = 0x100;
pub const TRAP_MSG_LEN: u32 = 0x104;
pub const TRAP_WORD_ADDR: u32 = 0x108;
pub const TRAP_WORD_LEN: u32 = 0x10C;
/// Next free heap byte for the bump allocator.
pub const HEAP_PTR: u32 = 0x110;
/// Data stack pointer (REPL, M2).
pub const DATA_STACK_PTR: u32 = 0x114;
/// Browser doorbell: the worker stores 0 and waits here in `ring_enter`; the main thread services the ring, stores 1 and notifies.
pub const DOORBELL: u32 = 0x118;
/// Unwind/rewind mode of the running process (M12): 0 normal, 1 unwinding,
/// 2 rewinding. A cell, not a wasm global, so every REPL step module sees it.
pub const UNWIND_MODE: u32 = 0x11C;
pub const UNWIND_OFF: i32 = 0;
pub const UNWINDING: i32 = 1;
pub const REWINDING: i32 = 2;
/// WASI builds only: two iovecs for `fd_read`/`fd_write` (16 bytes).
pub const WASI_IOVEC: u32 = 0x120;
/// WASI builds only: a call's out-parameter (bytes moved, an opened fd).
pub const WASI_RESULT: u32 = 0x130;
/// WASI builds only: whether `/dev/time` has been read since it was opened.
pub const WASI_TIME_DONE: u32 = 0x134;

/// I/O ring: 64 KiB to 128 KiB.
pub const RING_BASE: u32 = 0x1_0000;
pub const SQ_HEAD: u32 = RING_BASE;
pub const SQ_TAIL: u32 = RING_BASE + 4;
pub const CQ_HEAD: u32 = RING_BASE + 8;
pub const CQ_TAIL: u32 = RING_BASE + 12;
pub const RING_ENTRIES: u32 = 256;
pub const SQ_BASE: u32 = RING_BASE + 0x100;
pub const SQE_SIZE: u32 = 32;
pub const CQ_BASE: u32 = SQ_BASE + RING_ENTRIES * SQE_SIZE;
pub const CQE_SIZE: u32 = 16;
pub const RING_END: u32 = 0x2_0000;

/// Submission entry field offsets.
pub const SQE_OP: u32 = 0;
pub const SQE_USER: u32 = 4;
pub const SQE_A0: u32 = 8;
pub const SQE_A1: u32 = 12;
pub const SQE_A2: u32 = 16;
/// Completion entry field offsets.
pub const CQE_USER: u32 = 0;
pub const CQE_RESULT: u32 = 4;

/// Ring opcodes.
pub const OP_OPEN: i32 = 1;
pub const OP_READ: i32 = 2;
pub const OP_WRITE: i32 = 3;
pub const OP_CLOSE: i32 = 4;
/// Start a process: a0 = the funcref-table slot of the closure's code; the
/// closure value itself is in the imported mutable `anyref` global
/// `wack.spawn` (see `IMPORT_SPAWN`). Result 0.
pub const OP_SPAWN: i32 = 5;
/// Make a channel. Result: a new channel id (from 1, in creation order) with
/// one sender.
pub const OP_CHAN_MAKE: i32 = 6;
/// a0 = channel id: one more sender. Result 0.
pub const OP_CHAN_SENDER: i32 = 7;
/// a0 = channel id. The sender has pushed its value onto the channel's queue
/// in wasm; the process parks until a receiver takes that value. Result 0, or
/// `E_CLOSED` when the channel is closed.
pub const OP_CHAN_SEND: i32 = 8;
/// a0 = channel id. Parks until a value is queued or the channel is closed
/// and drained. Result 1: take the front value from the queue; 0: closed and
/// drained.
pub const OP_CHAN_RECV: i32 = 9;
/// a0 = channel id: one sender fewer; closed at zero. Result 0, or
/// `E_CLOSED` when it already had no senders.
pub const OP_CHAN_CLOSE: i32 = 10;
/// a0 = address of `a1` little-endian i32 channel ids, a1 = their count.
/// Parks until one of them has a value or is closed and drained. Result
/// `2 * i + v` for arm `i`: `v` = 1 take a value from channel `i`, 0 that
/// channel is closed and drained. The lowest ready arm wins.
pub const OP_ALT: i32 = 11;

/// Data stack: 128 KiB to 1 MiB.
pub const DATA_STACK_BASE: u32 = 0x2_0000;
pub const DATA_STACK_END: u32 = 0x10_0000;
/// Bytes per data stack slot: one per wasm value (i32/f32 in the low 4 bytes; `str` and `array` take two, addr then len).
pub const STACK_SLOT: u32 = 8;

/// Read-only literals start here; the heap follows them.
pub const LITERALS_BASE: u32 = 0x10_0000;

/// Initial memory: 4 MiB.
pub const INITIAL_PAGES: u64 = 64;
/// Maximum of the browser's shared memory (64 MiB); shared memories must declare one.
pub const SHARED_MAX_PAGES: u64 = 1024;

/// Host import module and doorbell name.
pub const IMPORT_MODULE: &str = "wack";
pub const IMPORT_RING_ENTER: &str = "ring_enter";
/// Shared memory import name of REPL step modules.
pub const IMPORT_MEMORY: &str = "memory";
/// Shared funcref table import name of REPL step modules.
pub const IMPORT_TABLE: &str = "table";
/// Shared anyref table of REPL step modules that use structs: a reference
/// on the memory data stack is a slot holding its own index into this table.
pub const IMPORT_REFS: &str = "refs";
/// The mutable `anyref` global a module imports from `wack` only when it uses
/// `spawn`: the closure of the process being started (`OP_SPAWN`).
pub const IMPORT_SPAWN: &str = "spawn";
/// The mutable `anyref` global a transformed module imports from `wack`: the
/// head of the frame chain of the running process.
pub const IMPORT_FRAMES: &str = "frames";
/// Table index of `wack.refs` in a step module (0 is `wack.table`).
pub const REFS_TABLE: u32 = 1;
pub const EXPORT_MEMORY: &str = "memory";
/// A transformed module (M12) exports its funcref table, for the native
/// driver to start a process at its closure's slot.
pub const EXPORT_TABLE: &str = "table";

/// I/O error codes (negative i32).
pub const E_NOT_FOUND: i32 = -1;
pub const E_PERMISSION: i32 = -2;
pub const E_NOT_SUPPORTED: i32 = -3;
pub const E_IO: i32 = -4;
pub const E_BAD_HANDLE: i32 = -5;
/// A malformed request written to a handle (a `/net/http` header block).
pub const E_MALFORMED: i32 = -6;
/// A closed channel.
pub const E_CLOSED: i32 = -7;

/// Open modes.
pub const MODE_READ: i32 = 0;
pub const MODE_WRITE: i32 = 1;
pub const MODE_APPEND: i32 = 2;
pub const MODE_READ_WRITE: i32 = 3;

const _: () = {
    assert!(CQ_BASE + RING_ENTRIES * CQE_SIZE <= RING_END);
    assert!(TRAP_MSG_ADDR < RESERVED_END);
    assert!(DOORBELL < RESERVED_END);
    assert!(DOORBELL + 4 <= WASI_IOVEC);
    assert!(UNWIND_MODE + 4 <= WASI_IOVEC);
    assert!(WASI_TIME_DONE + 4 <= RESERVED_END);
};

/// Name/value pairs a JavaScript host needs; negative codes are cast to `u32`.
pub fn constants() -> Vec<(&'static str, u32)> {
    vec![
        ("TRAP_MSG_ADDR", TRAP_MSG_ADDR),
        ("TRAP_MSG_LEN", TRAP_MSG_LEN),
        ("TRAP_WORD_ADDR", TRAP_WORD_ADDR),
        ("TRAP_WORD_LEN", TRAP_WORD_LEN),
        ("HEAP_PTR", HEAP_PTR),
        ("DATA_STACK_PTR", DATA_STACK_PTR),
        ("DOORBELL", DOORBELL),
        ("UNWIND_MODE", UNWIND_MODE),
        ("UNWIND_OFF", UNWIND_OFF as u32),
        ("UNWINDING", UNWINDING as u32),
        ("REWINDING", REWINDING as u32),
        ("RING_BASE", RING_BASE),
        ("SQ_HEAD", SQ_HEAD),
        ("SQ_TAIL", SQ_TAIL),
        ("CQ_HEAD", CQ_HEAD),
        ("CQ_TAIL", CQ_TAIL),
        ("RING_ENTRIES", RING_ENTRIES),
        ("SQ_BASE", SQ_BASE),
        ("SQE_SIZE", SQE_SIZE),
        ("CQ_BASE", CQ_BASE),
        ("CQE_SIZE", CQE_SIZE),
        ("SQE_OP", SQE_OP),
        ("SQE_USER", SQE_USER),
        ("SQE_A0", SQE_A0),
        ("SQE_A1", SQE_A1),
        ("SQE_A2", SQE_A2),
        ("CQE_USER", CQE_USER),
        ("CQE_RESULT", CQE_RESULT),
        ("OP_OPEN", OP_OPEN as u32),
        ("OP_READ", OP_READ as u32),
        ("OP_WRITE", OP_WRITE as u32),
        ("OP_CLOSE", OP_CLOSE as u32),
        ("OP_SPAWN", OP_SPAWN as u32),
        ("OP_CHAN_MAKE", OP_CHAN_MAKE as u32),
        ("OP_CHAN_SENDER", OP_CHAN_SENDER as u32),
        ("OP_CHAN_SEND", OP_CHAN_SEND as u32),
        ("OP_CHAN_RECV", OP_CHAN_RECV as u32),
        ("OP_CHAN_CLOSE", OP_CHAN_CLOSE as u32),
        ("OP_ALT", OP_ALT as u32),
        ("DATA_STACK_BASE", DATA_STACK_BASE),
        ("DATA_STACK_END", DATA_STACK_END),
        ("LITERALS_BASE", LITERALS_BASE),
        ("STACK_SLOT", STACK_SLOT),
        ("REFS_TABLE", REFS_TABLE),
        ("INITIAL_PAGES", INITIAL_PAGES as u32),
        ("SHARED_MAX_PAGES", SHARED_MAX_PAGES as u32),
        ("E_NOT_FOUND", E_NOT_FOUND as u32),
        ("E_PERMISSION", E_PERMISSION as u32),
        ("E_NOT_SUPPORTED", E_NOT_SUPPORTED as u32),
        ("E_IO", E_IO as u32),
        ("E_BAD_HANDLE", E_BAD_HANDLE as u32),
        ("E_MALFORMED", E_MALFORMED as u32),
        ("E_CLOSED", E_CLOSED as u32),
        ("MODE_READ", MODE_READ as u32),
        ("MODE_WRITE", MODE_WRITE as u32),
        ("MODE_APPEND", MODE_APPEND as u32),
        ("MODE_READ_WRITE", MODE_READ_WRITE as u32),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_for_hosts() {
        let c = constants();
        assert!(c.contains(&("DATA_STACK_PTR", 0x114)));
        assert!(c.contains(&("DOORBELL", 0x118)));
        assert!(c.contains(&("UNWIND_MODE", 0x11C)));
        assert!(c.contains(&("OP_ALT", 11)));
        assert!(c.contains(&("E_CLOSED", -7i32 as u32)));
        const { assert!(DOORBELL < RESERVED_END) };
    }
}
