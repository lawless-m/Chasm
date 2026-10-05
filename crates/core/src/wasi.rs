//! The WASI export mapping: the four host words onto five preview1 imports
//! and a path table. `/dev/cons` is fds 0 and 1, `/dev/time` is
//! `clock_time_get`, `/file/<path>` is `path_open` under the first preopened
//! directory (fd 3), and any other path is not found. The ring helper's
//! signature is kept, so compiled words are the same in both modes.

use wasm_encoder::{BlockType, Function, Instruction as I, MemArg, ValType};

use crate::check::Ctx;
use crate::layout as L;

pub const MODULE: &str = "wasi_snapshot_preview1";

// Import indices, in import order.
const FD_WRITE: u32 = 0;
const FD_READ: u32 = 1;
const FD_CLOSE: u32 = 2;
const PATH_OPEN: u32 = 3;
const CLOCK_TIME_GET: u32 = 4;

/// The handle `/dev/time` opens to; never a real fd.
const TIME_HANDLE: i32 = 0x4000_0000;
/// The first preopened directory.
const PREOPEN_FD: i32 = 3;
const LOOKUP_SYMLINK_FOLLOW: i32 = 1;
const RIGHTS: i64 = (1 << 1) | (1 << 2) | (1 << 6) | (1 << 21);

/// The preview1 imports, in index order: (module, name, type index).
pub fn imports(ctx: &mut Ctx) -> Vec<(&'static str, &'static str, u32)> {
    use ValType::{I32, I64};
    let rw = ctx.intern_type(vec![I32; 4], vec![I32]);
    let close = ctx.intern_type(vec![I32], vec![I32]);
    let open = ctx.intern_type(vec![I32, I32, I32, I32, I32, I64, I64, I32, I32], vec![I32]);
    let clock = ctx.intern_type(vec![I32, I64, I32], vec![I32]);
    vec![
        (MODULE, "fd_write", rw),
        (MODULE, "fd_read", rw),
        (MODULE, "fd_close", close),
        (MODULE, "path_open", open),
        (MODULE, "clock_time_get", clock),
    ]
}

fn at(offset: u32) -> MemArg {
    MemArg {
        offset: offset as u64,
        align: 0,
        memory_index: 0,
    }
}

/// Return the result in local `r` mapped from a WASI errno to a Whackford code.
fn map_errno(code: &mut Vec<I<'static>>, r: u32) {
    for (errno, wack) in [
        (44, L::E_NOT_FOUND),
        (2, L::E_PERMISSION),
        (8, L::E_BAD_HANDLE),
        (58, L::E_NOT_SUPPORTED),
    ] {
        code.extend([
            I::LocalGet(r),
            I::I32Const(errno),
            I::I32Eq,
            I::If(BlockType::Empty),
            I::I32Const(wack),
            I::Return,
            I::End,
        ]);
    }
    code.extend([I::I32Const(L::E_IO), I::Return]);
}

/// Does the path at local 0, of length local 1, equal a 9-byte name?
fn is_name9(code: &mut Vec<I<'static>>, name: &[u8; 9]) {
    let head = i64::from_le_bytes(name[..8].try_into().unwrap());
    code.extend([
        I::LocalGet(1),
        I::I32Const(9),
        I::I32Eq,
        I::If(BlockType::Result(ValType::I32)),
        I::LocalGet(0),
        I::I64Load(at(0)),
        I::I64Const(head),
        I::I64Eq,
        I::LocalGet(0),
        I::I32Load8U(at(8)),
        I::I32Const(name[8] as i32),
        I::I32Eq,
        I::I32And,
        I::Else,
        I::I32Const(0),
        I::End,
    ]);
}

/// A read or write through one iovec: `call(fd, iovs, 1, nread)`.
fn iovec_call(code: &mut Vec<I<'static>>, func: u32, r: u32) {
    code.extend([
        I::I32Const(0),
        I::LocalGet(1),
        I::I32Store(at(L::WASI_IOVEC)),
        I::I32Const(0),
        I::LocalGet(2),
        I::I32Store(at(L::WASI_IOVEC + 4)),
        I::LocalGet(0),
        I::I32Const(L::WASI_IOVEC as i32),
        I::I32Const(1),
        I::I32Const(L::WASI_RESULT as i32),
        I::Call(func),
        I::LocalTee(r),
        I::I32Eqz,
        I::If(BlockType::Empty),
        I::I32Const(0),
        I::I32Load(at(L::WASI_RESULT)),
        I::Return,
        I::End,
    ]);
    map_errno(code, r);
}

/// `ring(a0, a1, a2, op) -> result` over WASI.
pub fn rt_ring() -> Function {
    // params a0=0 a1=1 a2=2 op=3; local r=4
    let r = 4;
    let mut f = Function::new([(1, ValType::I32)]);
    let mut c: Vec<I<'static>> = Vec::new();
    let op = |c: &mut Vec<I<'static>>, op: i32| {
        c.extend([
            I::LocalGet(3),
            I::I32Const(op),
            I::I32Eq,
            I::If(BlockType::Empty),
        ])
    };

    // open(path addr, path len, mode)
    op(&mut c, L::OP_OPEN);
    is_name9(&mut c, b"/dev/cons");
    c.extend([
        I::If(BlockType::Empty),
        I::I32Const(0),
        I::I32Const(1),
        I::LocalGet(2),
        I::I32Eqz,
        I::Select,
        I::Return,
        I::End,
    ]);
    is_name9(&mut c, b"/dev/time");
    c.extend([
        I::If(BlockType::Empty),
        I::LocalGet(2),
        I::If(BlockType::Empty),
        I::I32Const(L::E_PERMISSION),
        I::Return,
        I::End,
        I::I32Const(0),
        I::I32Const(0),
        I::I32Store(at(L::WASI_TIME_DONE)),
        I::I32Const(TIME_HANDLE),
        I::Return,
        I::End,
    ]);
    let file = i32::from_le_bytes(*b"/fil");
    let slash = u16::from_le_bytes(*b"e/") as i32;
    c.extend([
        I::LocalGet(1),
        I::I32Const(6),
        I::I32GeS,
        I::If(BlockType::Result(ValType::I32)),
        I::LocalGet(0),
        I::I32Load(at(0)),
        I::I32Const(file),
        I::I32Eq,
        I::LocalGet(0),
        I::I32Load16U(at(4)),
        I::I32Const(slash),
        I::I32Eq,
        I::I32And,
        I::Else,
        I::I32Const(0),
        I::End,
        I::If(BlockType::Empty),
        I::I32Const(PREOPEN_FD),
        I::I32Const(LOOKUP_SYMLINK_FOLLOW),
        I::LocalGet(0),
        I::I32Const(6),
        I::I32Add,
        I::LocalGet(1),
        I::I32Const(6),
        I::I32Sub,
        // oflags: write is create | truncate, append is create
        I::LocalGet(2),
        I::I32Const(L::MODE_WRITE),
        I::I32Eq,
        I::I32Const(9),
        I::I32Mul,
        I::LocalGet(2),
        I::I32Const(L::MODE_APPEND),
        I::I32Eq,
        I::I32Or,
        I::I64Const(RIGHTS),
        I::I64Const(RIGHTS),
        // fdflags: append
        I::LocalGet(2),
        I::I32Const(L::MODE_APPEND),
        I::I32Eq,
        I::I32Const(L::WASI_RESULT as i32),
        I::Call(PATH_OPEN),
        I::LocalTee(r),
        I::I32Eqz,
        I::If(BlockType::Empty),
        I::I32Const(0),
        I::I32Load(at(L::WASI_RESULT)),
        I::Return,
        I::End,
    ]);
    map_errno(&mut c, r);
    c.extend([I::End, I::I32Const(L::E_NOT_FOUND), I::Return, I::End]);

    // read(handle, buf, len)
    op(&mut c, L::OP_READ);
    c.extend([
        I::LocalGet(0),
        I::I32Const(TIME_HANDLE),
        I::I32Eq,
        I::If(BlockType::Empty),
        I::I32Const(0),
        I::I32Load(at(L::WASI_TIME_DONE)),
        I::If(BlockType::Empty),
        I::I32Const(0),
        I::Return,
        I::End,
        I::LocalGet(2),
        I::I32Const(8),
        I::I32LtS,
        I::If(BlockType::Empty),
        I::I32Const(L::E_IO),
        I::Return,
        I::End,
        I::I32Const(0), // realtime
        I::I64Const(1),
        I::LocalGet(1),
        I::Call(CLOCK_TIME_GET),
        I::LocalTee(r),
        I::I32Eqz,
        I::If(BlockType::Empty),
        I::I32Const(0),
        I::I32Const(1),
        I::I32Store(at(L::WASI_TIME_DONE)),
        I::I32Const(8),
        I::Return,
        I::End,
    ]);
    map_errno(&mut c, r);
    c.push(I::End);
    iovec_call(&mut c, FD_READ, r);
    c.push(I::End);

    // write(handle, buf, len)
    op(&mut c, L::OP_WRITE);
    iovec_call(&mut c, FD_WRITE, r);
    c.push(I::End);

    // close(handle)
    op(&mut c, L::OP_CLOSE);
    c.extend([
        I::LocalGet(0),
        I::I32Const(2),
        I::I32LeU,
        I::LocalGet(0),
        I::I32Const(TIME_HANDLE),
        I::I32Eq,
        I::I32Or,
        I::If(BlockType::Empty),
        I::I32Const(0),
        I::Return,
        I::End,
        I::LocalGet(0),
        I::Call(FD_CLOSE),
        I::LocalTee(r),
        I::I32Eqz,
        I::If(BlockType::Empty),
        I::I32Const(0),
        I::Return,
        I::End,
    ]);
    map_errno(&mut c, r);
    c.extend([I::End, I::I32Const(L::E_NOT_SUPPORTED), I::End]);

    for i in &c {
        f.instruction(i);
    }
    f
}

/// `trap(msg, len, word, wlen)`: record the trap as the ring host does, print
/// the message and a newline to stderr, then stop.
pub fn rt_trap(newline: i32) -> Function {
    let mut f = Function::new([]);
    let cell = |offset| MemArg {
        offset: offset as u64,
        align: 2,
        memory_index: 0,
    };
    for (i, c) in [
        L::TRAP_MSG_ADDR,
        L::TRAP_MSG_LEN,
        L::TRAP_WORD_ADDR,
        L::TRAP_WORD_LEN,
    ]
    .into_iter()
    .enumerate()
    {
        f.instruction(&I::I32Const(0));
        f.instruction(&I::LocalGet(i as u32));
        f.instruction(&I::I32Store(cell(c)));
    }
    for i in [
        I::I32Const(0),
        I::LocalGet(0),
        I::I32Store(at(L::WASI_IOVEC)),
        I::I32Const(0),
        I::LocalGet(1),
        I::I32Store(at(L::WASI_IOVEC + 4)),
        I::I32Const(0),
        I::I32Const(newline),
        I::I32Store(at(L::WASI_IOVEC + 8)),
        I::I32Const(0),
        I::I32Const(1),
        I::I32Store(at(L::WASI_IOVEC + 12)),
        I::I32Const(2),
        I::I32Const(L::WASI_IOVEC as i32),
        I::I32Const(2),
        I::I32Const(L::WASI_RESULT as i32),
        I::Call(FD_WRITE),
        I::Drop,
        I::Unreachable,
        I::End,
    ] {
        f.instruction(&i);
    }
    f
}
