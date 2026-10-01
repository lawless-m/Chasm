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

/// Data stack: 128 KiB to 1 MiB.
pub const DATA_STACK_BASE: u32 = 0x2_0000;
pub const DATA_STACK_END: u32 = 0x10_0000;

/// Read-only literals start here; the heap follows them.
pub const LITERALS_BASE: u32 = 0x10_0000;

/// Initial memory: 4 MiB.
pub const INITIAL_PAGES: u64 = 64;

/// Host import module and doorbell name.
pub const IMPORT_MODULE: &str = "chasm";
pub const IMPORT_RING_ENTER: &str = "ring_enter";
pub const EXPORT_MEMORY: &str = "memory";

/// I/O error codes (negative i32).
pub const E_NOT_FOUND: i32 = -1;
pub const E_PERMISSION: i32 = -2;
pub const E_NOT_SUPPORTED: i32 = -3;
pub const E_IO: i32 = -4;
pub const E_BAD_HANDLE: i32 = -5;

/// Open modes.
pub const MODE_READ: i32 = 0;
pub const MODE_WRITE: i32 = 1;
pub const MODE_APPEND: i32 = 2;
pub const MODE_READ_WRITE: i32 = 3;

const _: () = {
    assert!(CQ_BASE + RING_ENTRIES * CQE_SIZE <= RING_END);
    assert!(TRAP_MSG_ADDR < RESERVED_END);
};
