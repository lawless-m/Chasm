//! Assemble the wasm module: runtime helpers, word functions, table,
//! memory, literals.

use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, CustomSection, DataSection, ElementSection, Elements,
    EntityType, ExportKind, ExportSection, Function, FunctionSection, ImportSection,
    Instruction as I, MemArg, MemorySection, MemoryType, Module, NameMap, NameSection, RefType,
    TableSection, TableType, TypeSection, ValType,
};

use crate::check::{Ctx, Word, FN_ALLOC, FN_RING, FN_TRAP};
use crate::layout as L;

fn m(offset: u32) -> MemArg {
    MemArg {
        offset: offset as u64,
        align: 2,
        memory_index: 0,
    }
}

/// `alloc(n) -> addr`: 8-byte aligned bump allocation, zero-filled, growing memory as needed.
fn rt_alloc(ctx: &mut Ctx) -> Function {
    // params: n=0; locals: p=1, new=2
    let mut f = Function::new([(2, ValType::I32)]);
    let (oom_a, oom_l) = ctx.intern_str("out of memory");
    let (w_a, w_l) = ctx.intern_str("mem.alloc");
    let ins = [
        I::I32Const(0),
        I::I32Load(m(L::HEAP_PTR)),
        I::I32Const(7),
        I::I32Add,
        I::I32Const(-8),
        I::I32And,
        I::LocalTee(1),
        I::LocalGet(0),
        I::I32Add,
        I::LocalSet(2),
        // if new > memory bytes: grow
        I::LocalGet(2),
        I::MemorySize(0),
        I::I32Const(16),
        I::I32Shl,
        I::I32GtU,
        I::If(BlockType::Empty),
        I::LocalGet(2),
        I::MemorySize(0),
        I::I32Const(16),
        I::I32Shl,
        I::I32Sub,
        I::I32Const(0xFFFF),
        I::I32Add,
        I::I32Const(16),
        I::I32ShrU,
        I::MemoryGrow(0),
        I::I32Const(-1),
        I::I32Eq,
        I::If(BlockType::Empty),
        I::I32Const(oom_a),
        I::I32Const(oom_l),
        I::I32Const(w_a),
        I::I32Const(w_l),
        I::Call(FN_TRAP),
        I::End,
        I::End,
        I::I32Const(0),
        I::LocalGet(2),
        I::I32Store(m(L::HEAP_PTR)),
        I::LocalGet(1),
        I::I32Const(0),
        I::LocalGet(0),
        I::MemoryFill(0),
        I::LocalGet(1),
        I::End,
    ];
    for i in &ins {
        f.instruction(i);
    }
    f
}

/// `trap(msg_addr, msg_len, word_addr, word_len)`: record the message, then trap.
fn rt_trap() -> Function {
    let mut f = Function::new([]);
    for (i, cell) in [
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
        f.instruction(&I::I32Store(m(cell)));
    }
    f.instruction(&I::Unreachable);
    f.instruction(&I::End);
    f
}

/// `ring(a0, a1, a2, op) -> result`: submit one request, ring the doorbell,
/// take one completion.
fn rt_ring() -> Function {
    // params a0=0 a1=1 a2=2 op=3; locals t=4 e=5
    let mut f = Function::new([(2, ValType::I32)]);
    let ins = [
        I::I32Const(0),
        I::I32Load(m(L::SQ_TAIL)),
        I::LocalTee(4),
        I::I32Const((L::RING_ENTRIES - 1) as i32),
        I::I32And,
        I::I32Const(5), // * SQE_SIZE (32)
        I::I32Shl,
        I::I32Const(L::SQ_BASE as i32),
        I::I32Add,
        I::LocalSet(5),
        I::LocalGet(5),
        I::LocalGet(3),
        I::I32Store(m(L::SQE_OP)),
        I::LocalGet(5),
        I::LocalGet(4),
        I::I32Store(m(L::SQE_USER)),
        I::LocalGet(5),
        I::LocalGet(0),
        I::I32Store(m(L::SQE_A0)),
        I::LocalGet(5),
        I::LocalGet(1),
        I::I32Store(m(L::SQE_A1)),
        I::LocalGet(5),
        I::LocalGet(2),
        I::I32Store(m(L::SQE_A2)),
        I::I32Const(0),
        I::LocalGet(4),
        I::I32Const(1),
        I::I32Add,
        I::I32Store(m(L::SQ_TAIL)),
        I::Call(crate::check::FN_RING_ENTER),
        // completion
        I::I32Const(0),
        I::I32Load(m(L::CQ_HEAD)),
        I::LocalTee(4),
        I::I32Const((L::RING_ENTRIES - 1) as i32),
        I::I32And,
        I::I32Const(4), // * CQE_SIZE (16)
        I::I32Shl,
        I::I32Const(L::CQ_BASE as i32),
        I::I32Add,
        I::I32Load(m(L::CQE_RESULT)),
        I::I32Const(0),
        I::LocalGet(4),
        I::I32Const(1),
        I::I32Add,
        I::I32Store(m(L::CQ_HEAD)),
        I::End,
    ];
    for i in &ins {
        f.instruction(i);
    }
    f
}

pub struct ModuleOptions {
    /// Export `__test_N` thunks: (export name, word id).
    pub test_exports: Vec<(String, usize)>,
}

pub fn assemble(ctx: &mut Ctx, opts: &ModuleOptions) -> Vec<u8> {
    let void = ctx.intern_type(vec![], vec![]);
    let t_alloc = ctx.intern_type(vec![ValType::I32], vec![ValType::I32]);
    let t_trap = ctx.intern_type(vec![ValType::I32; 4], vec![]);
    let t_ring = ctx.intern_type(vec![ValType::I32; 4], vec![ValType::I32]);

    let mut funcs = FunctionSection::new();
    let mut code = CodeSection::new();
    let mut names = NameMap::new();
    names.append(0, "chasm.ring_enter");

    let helpers = [
        (FN_ALLOC, t_alloc, rt_alloc(ctx), "rt.alloc"),
        (FN_TRAP, t_trap, rt_trap(), "rt.trap"),
        (FN_RING, t_ring, rt_ring(), "rt.ring"),
    ];
    for (idx, ty, f, name) in helpers {
        funcs.function(ty);
        code.function(&f);
        names.append(idx, name);
    }

    for id in 0..ctx.words.len() {
        let w = ctx.words[id].clone();
        let ty = ctx.intern_type(w.effect.wasm_params(), w.effect.wasm_results());
        funcs.function(ty);
        let f = match &w.body {
            Some(c) => {
                let mut f = Function::new_with_locals_types(c.locals.iter().copied());
                for i in &c.code {
                    f.instruction(i);
                }
                f.instruction(&I::End);
                f
            }
            None => {
                // Unresolved stub: trap with a message.
                let mut f = Function::new([]);
                let (ma, ml) = ctx.intern_str(&format!("unresolved word {}", w.name));
                let (wa, wl) = ctx.intern_str(&w.name);
                for i in [
                    I::I32Const(ma),
                    I::I32Const(ml),
                    I::I32Const(wa),
                    I::I32Const(wl),
                    I::Call(FN_TRAP),
                    I::Unreachable,
                    I::End,
                ] {
                    f.instruction(&i);
                }
                f
            }
        };
        code.function(&f);
        names.append(Word::func_index(id), &w.name);
    }

    let mut types = TypeSection::new();
    for (p, r) in &ctx.types {
        types.ty().function(p.iter().copied(), r.iter().copied());
    }

    let mut imports = ImportSection::new();
    imports.import(
        L::IMPORT_MODULE,
        L::IMPORT_RING_ENTER,
        EntityType::Function(void),
    );

    let nwords = ctx.words.len() as u64;
    let mut tables = TableSection::new();
    tables.table(TableType {
        element_type: RefType::FUNCREF,
        table64: false,
        minimum: nwords,
        maximum: Some(nwords),
        shared: false,
    });

    let heap_start = (L::LITERALS_BASE + ctx.literals.len() as u32 + 7) & !7;
    let pages = (L::INITIAL_PAGES).max((heap_start as u64).div_ceil(L::PAGE as u64) + 1);
    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: pages,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });

    let mut exports = ExportSection::new();
    exports.export(L::EXPORT_MEMORY, ExportKind::Memory, 0);
    let mut exported = std::collections::HashSet::new();
    for (id, w) in ctx.words.iter().enumerate() {
        if (w.export || w.name == "main")
            && w.kind == crate::check::WordKind::Named
            && exported.insert(w.name.clone())
        {
            exports.export(&w.name, ExportKind::Func, Word::func_index(id));
        }
    }
    for (name, id) in &opts.test_exports {
        exports.export(name, ExportKind::Func, Word::func_index(*id));
    }

    let mut elems = ElementSection::new();
    let fidx: Vec<u32> = (0..ctx.words.len()).map(Word::func_index).collect();
    if !fidx.is_empty() {
        elems.active(
            Some(0),
            &ConstExpr::i32_const(0),
            Elements::Functions(fidx.into()),
        );
    }

    let mut data = DataSection::new();
    data.active(
        0,
        &ConstExpr::i32_const(L::LITERALS_BASE as i32),
        ctx.literals.iter().copied(),
    );
    data.active(
        0,
        &ConstExpr::i32_const(L::HEAP_PTR as i32),
        heap_start.to_le_bytes(),
    );

    let mut name_sec = NameSection::new();
    name_sec.module("chasm");
    name_sec.functions(&names);

    let mut module = Module::new();
    module.section(&types);
    module.section(&imports);
    module.section(&funcs);
    module.section(&tables);
    module.section(&memories);
    module.section(&exports);
    module.section(&elems);
    module.section(&code);
    module.section(&data);
    module.section(&name_sec);
    let producers = CustomSection {
        name: "chasm-version".into(),
        data: env!("CARGO_PKG_VERSION").as_bytes().into(),
    };
    module.section(&producers);
    module.finish()
}
