//! Assemble the wasm module: runtime helpers, word functions, table,
//! memory, literals.

use wasm_encoder::{
    ArrayType, BlockType, CodeSection, CompositeInnerType, CompositeType, ConstExpr, CustomSection,
    DataSection, ElementSection, Elements, EntityType, ExportKind, ExportSection, FieldType,
    Function, FunctionSection, GlobalType, ImportSection, Instruction as I, MemArg, MemorySection,
    MemoryType, Module, NameMap, NameSection, RefType, StorageType, StructType, SubType,
    TableSection, TableType, TypeSection, ValType,
};

use crate::check::{
    Ctx, Member, TypeDef, Unwind, Word, WordId, WordKind, FN_ALLOC, FN_RING, FN_TRAP,
};
use crate::layout as L;
use crate::types::ref_ty;

fn m(offset: u32) -> MemArg {
    MemArg {
        offset: offset as u64,
        align: 2,
        memory_index: 0,
    }
}

/// `alloc(n) -> addr`: 8-byte aligned bump allocation, zero-filled, growing memory as needed.
fn rt_alloc(ctx: &mut Ctx, shift: u32) -> Function {
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
        I::Call(FN_TRAP + shift),
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

/// Submit one request (params a0=0 a1=1 a2=2 op=3; locals t=4 e=5) and
/// ring the doorbell.
fn ring_submit() -> Vec<I<'static>> {
    vec![
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
    ]
}

/// Take one completion: its result is left on the stack.
fn ring_complete() -> Vec<I<'static>> {
    vec![
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
    ]
}

/// `[mode] == value`.
fn unwind_mode_is(value: i32) -> [I<'static>; 4] {
    [
        I::I32Const(0),
        I::I32Load(m(L::UNWIND_MODE)),
        I::I32Const(value),
        I::I32Eq,
    ]
}

/// `ring(a0, a1, a2, op) -> result`: submit one request, ring the doorbell,
/// take one completion.
fn rt_ring() -> Function {
    let mut f = Function::new([(2, ValType::I32)]);
    for i in ring_submit()
        .iter()
        .chain(&ring_complete())
        .chain(&[I::End])
    {
        f.instruction(i);
    }
    f
}

/// `rt_ring` in a transformed module (M12): rewinding, it is the call the
/// process parked in, so it takes the completion the host has written
/// without submitting; when `ring_enter` returns unwinding, the process
/// parks and the result is a dummy.
fn rt_ring_unwind() -> Function {
    let mut f = Function::new([(2, ValType::I32)]);
    let mut ins: Vec<I> = unwind_mode_is(L::REWINDING).to_vec();
    ins.extend([
        I::If(BlockType::Empty),
        I::I32Const(0),
        I::I32Const(L::UNWIND_OFF),
        I::I32Store(m(L::UNWIND_MODE)),
    ]);
    ins.extend(ring_complete());
    ins.extend([I::Return, I::End]);
    ins.extend(ring_submit());
    ins.extend(unwind_mode_is(L::UNWINDING));
    ins.extend([I::If(BlockType::Empty), I::I32Const(0), I::Return, I::End]);
    ins.extend(ring_complete());
    ins.push(I::End);
    for i in &ins {
        f.instruction(i);
    }
    f
}

pub struct ModuleOptions {
    /// Export `__test_N` thunks: (export name, word id).
    pub test_exports: Vec<(String, usize)>,
    /// The words to emit; `None` emits every word. A word keeps its table
    /// slot (its id) either way, so function values stay valid.
    pub live: Option<std::collections::HashSet<usize>>,
    /// Import WASI preview1 in place of the ring host and export `_start`.
    pub wasi: bool,
}

/// The runtime helpers in function-index order: (index, type, body, name).
/// The runtime helpers, numbered after `shift` extra imports; over WASI, the
/// ring and trap helpers call preview1 instead.
fn runtime_helpers(
    ctx: &mut Ctx,
    shift: u32,
    wasi: bool,
) -> Vec<(u32, u32, Function, &'static str)> {
    let t_alloc = ctx.intern_type(vec![ValType::I32], vec![ValType::I32]);
    let t_trap = ctx.intern_type(vec![ValType::I32; 4], vec![]);
    let t_ring = ctx.intern_type(vec![ValType::I32; 4], vec![ValType::I32]);
    let (trap, ring) = if wasi {
        let (newline, _) = ctx.intern_str("\n");
        (crate::wasi::rt_trap(newline), crate::wasi::rt_ring())
    } else if ctx.unwind != Unwind::Off {
        (rt_trap(), rt_ring_unwind())
    } else {
        (rt_trap(), rt_ring())
    };
    vec![
        (FN_ALLOC + shift, t_alloc, rt_alloc(ctx, shift), "rt.alloc"),
        (FN_TRAP + shift, t_trap, trap, "rt.trap"),
        (FN_RING + shift, t_ring, ring, "rt.ring"),
    ]
}

/// A word's function: its compiled body, or a stub that traps as unresolved.
/// `remap` gives a word's function index from its id when words are left
/// out or `shift` extra imports move every index, so direct calls are
/// renumbered.
fn word_function(ctx: &mut Ctx, w: &Word, remap: Option<&[Option<u32>]>, shift: u32) -> Function {
    match &w.body {
        Some(c) => {
            let mut f = Function::new_with_locals_types(c.locals.iter().copied());
            for i in &c.code {
                match (i, remap) {
                    (I::Call(x), Some(m)) if *x >= crate::check::FIRST_WORD_FN => {
                        let id = (*x - crate::check::FIRST_WORD_FN) as usize;
                        f.instruction(&I::Call(m[id].expect("a live word calls only live words")));
                    }
                    (I::Call(x), _) if shift > 0 => {
                        f.instruction(&I::Call(x + shift));
                    }
                    _ => {
                        f.instruction(i);
                    }
                }
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
                I::Call(FN_TRAP + shift),
                I::Unreachable,
                I::End,
            ] {
                f.instruction(&i);
            }
            f
        }
    }
}

fn name_section(names: NameMap) -> NameSection {
    let mut name_sec = NameSection::new();
    name_sec.module("wack");
    name_sec.functions(&names);
    name_sec
}

fn version_section() -> CustomSection<'static> {
    CustomSection {
        name: "wack-version".into(),
        data: env!("CARGO_PKG_VERSION").as_bytes().into(),
    }
}

/// A mutable `anyref` global import (`wack.frames`, `wack.spawn`).
fn anyref_global() -> EntityType {
    EntityType::Global(GlobalType {
        val_type: ValType::Ref(RefType::ANYREF),
        mutable: true,
        shared: false,
    })
}

pub fn assemble(ctx: &mut Ctx, opts: &ModuleOptions) -> Vec<u8> {
    register_word_types(ctx, &(0..ctx.words.len()).collect::<Vec<_>>());
    let void = ctx.intern_type(vec![], vec![]);
    // Imported functions: (module, name, type). Everything after them is
    // numbered `shift` past the single-import layout of `check`.
    let import_fns: Vec<(&str, &str, u32)> = if opts.wasi {
        crate::wasi::imports(ctx)
    } else {
        vec![(L::IMPORT_MODULE, L::IMPORT_RING_ENTER, void)]
    };
    let shift = import_fns.len() as u32 - 1;

    let mut funcs = FunctionSection::new();
    let mut code = CodeSection::new();
    let mut names = NameMap::new();
    for (k, (module, name, _)) in import_fns.iter().enumerate() {
        names.append(k as u32, &format!("{module}.{name}"));
    }

    for (idx, ty, f, name) in runtime_helpers(ctx, shift, opts.wasi) {
        funcs.function(ty);
        code.function(&f);
        names.append(idx, name);
    }

    // A generic template is never emitted: callers reach its instances.
    let kept: Vec<usize> = (0..ctx.words.len())
        .filter(|&id| ctx.words[id].generic.is_none())
        .filter(|id| opts.live.as_ref().is_none_or(|l| l.contains(id)))
        .collect();
    let mut index: Vec<Option<u32>> = vec![None; ctx.words.len()];
    for (k, &id) in kept.iter().enumerate() {
        index[id] = Some(Word::func_index(k) + shift);
    }
    let remap = (kept.len() != ctx.words.len() || shift > 0).then_some(index.as_slice());
    for &id in &kept {
        let w = ctx.words[id].clone();
        let ty = ctx.func_type(&w.effect, w.takes_env());
        funcs.function(ty);
        let f = word_function(ctx, &w, remap, shift);
        code.function(&f);
        names.append(index[id].unwrap(), &w.name);
    }
    // `_start` for WASI runtimes: calls `main`.
    let main_fn = ctx
        .by_name
        .get("main")
        .and_then(|&id| index.get(id).copied().flatten());
    let start_fn = match main_fn {
        Some(main) if opts.wasi => {
            let at = Word::func_index(kept.len()) + shift;
            funcs.function(void);
            let mut f = Function::new([]);
            f.instruction(&I::Call(main));
            f.instruction(&I::End);
            code.function(&f);
            names.append(at, "_start");
            Some(at)
        }
        _ => None,
    };

    let types = type_section(ctx);

    let mut imports = ImportSection::new();
    for (module, name, ty) in &import_fns {
        imports.import(module, name, EntityType::Function(*ty));
    }
    // Only a word kept in the module that spawns needs the global.
    let spawns = kept.iter().any(|&id| {
        ctx.words[id]
            .body
            .as_ref()
            .is_some_and(|c| crate::check::submits(c, |op| op == L::OP_SPAWN))
    });
    // A transformed module imports `wack.frames` (global 0) and, since a
    // transformed program uses processes, `wack.spawn` (global 1).
    let unwind = ctx.unwind != Unwind::Off;
    if unwind {
        imports.import(L::IMPORT_MODULE, L::IMPORT_FRAMES, anyref_global());
    }
    if spawns || unwind {
        imports.import(L::IMPORT_MODULE, L::IMPORT_SPAWN, anyref_global());
    }

    // Templates made last (prelude generics no word used) take no slot.
    let trailing = ctx
        .words
        .iter()
        .rev()
        .take_while(|w| w.generic.is_some() && w.kind == crate::check::WordKind::Named)
        .count();
    let nwords = (ctx.words.len() - trailing) as u64;
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
    if unwind {
        exports.export(L::EXPORT_TABLE, ExportKind::Table, 0);
    }
    let mut exported = std::collections::HashSet::new();
    for (id, w) in ctx.words.iter().enumerate() {
        if (w.export || w.name == "main")
            && w.kind == crate::check::WordKind::Named
            && exported.insert(w.name.clone())
        {
            exports.export(&w.name, ExportKind::Func, index[id].unwrap());
        }
    }
    for (name, id) in &opts.test_exports {
        exports.export(name, ExportKind::Func, index[*id].unwrap());
    }
    if let Some(at) = start_fn {
        exports.export("_start", ExportKind::Func, at);
    }

    // Table slot `id` holds word `id`; a left-out word's slot stays null.
    let mut elems = ElementSection::new();
    let mut id = 0;
    while id < nwords as usize {
        if index[id].is_none() {
            id += 1;
            continue;
        }
        let start = id;
        let mut run = Vec::new();
        while let Some(Some(f)) = index.get(id) {
            run.push(*f);
            id += 1;
        }
        elems.active(
            Some(0),
            &ConstExpr::i32_const(start as i32),
            Elements::Functions(run.into()),
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

    let name_sec = name_section(names);

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
    module.section(&version_section());
    module.finish()
}

/// The type section. Each struct is one rec group: the struct type, then its
/// GC array type, so a struct may hold itself or an array of itself; types
/// in other groups are distinct, and only earlier groups can be referenced.
fn type_section(ctx: &Ctx) -> TypeSection {
    let sub = |inner, is_final: bool, supertype: Option<u32>| SubType {
        is_final,
        supertype_idxs: supertype.into_iter().collect(),
        composite_type: CompositeType {
            inner,
            shared: false,
            descriptor: None,
            describes: None,
        },
    };
    let field = |vt: ValType| FieldType {
        element_type: StorageType::Val(vt),
        mutable: true,
    };
    let mut types = TypeSection::new();
    for t in &ctx.types {
        match t {
            TypeDef::Func(p, r) => {
                types.ty().function(p.iter().copied(), r.iter().copied());
            }
            TypeDef::Rec(members) => {
                let subs: Vec<SubType> = members
                    .iter()
                    .map(|m| match m {
                        Member::Struct {
                            fields,
                            is_final,
                            supertype,
                        } => sub(
                            CompositeInnerType::Struct(StructType {
                                fields: fields.iter().map(|&vt| field(vt)).collect(),
                            }),
                            *is_final,
                            *supertype,
                        ),
                        Member::Array(e) => sub(
                            CompositeInnerType::Array(ArrayType(field(ref_ty(*e)))),
                            true,
                            None,
                        ),
                    })
                    .collect();
                types.ty().rec(subs);
            }
            TypeDef::Slot => {}
        }
    }
    types
}

/// Export name of a word's function in a REPL step module.
pub fn export_name(id: WordId) -> String {
    format!("w{id}")
}

/// Assemble a REPL step module holding the functions of `ids`. It imports
/// the doorbell, the shared memory and the shared table, carries its own
/// copy of the runtime helpers, and exports each word as `w<id>`; the host
/// installs them in the table at slot = word id. No data, memory, table or
/// element sections: the host places the step's literals.
/// Register the concrete struct and union types in the effects of `ids`, so
/// their function types lower to the real reference types.
fn register_word_types(ctx: &mut Ctx, ids: &[WordId]) {
    for &id in ids {
        let e = ctx.words[id].effect.clone();
        ctx.register_effect(&e);
    }
}

pub fn assemble_step(ctx: &mut Ctx, ids: &[WordId], shared_memory: bool) -> Vec<u8> {
    register_word_types(ctx, ids);
    let void = ctx.intern_type(vec![], vec![]);

    let mut funcs = FunctionSection::new();
    let mut code = CodeSection::new();
    let mut names = NameMap::new();
    names.append(0, "wack.ring_enter");

    for (idx, ty, f, name) in runtime_helpers(ctx, 0, false) {
        funcs.function(ty);
        code.function(&f);
        names.append(idx, name);
    }

    let mut exports = ExportSection::new();
    for (k, &id) in ids.iter().enumerate() {
        let w = ctx.words[id].clone();
        let ty = if w.kind == WordKind::Line {
            void
        } else {
            ctx.func_type(&w.effect, w.takes_env())
        };
        funcs.function(ty);
        let f = word_function(ctx, &w, None, 0);
        code.function(&f);
        let index = crate::check::FIRST_WORD_FN + k as u32;
        names.append(index, &w.name);
        exports.export(&export_name(id), ExportKind::Func, index);
    }

    let types = type_section(ctx);

    let memory = if shared_memory {
        MemoryType {
            minimum: L::INITIAL_PAGES,
            maximum: Some(L::SHARED_MAX_PAGES),
            memory64: false,
            shared: true,
            page_size_log2: None,
        }
    } else {
        MemoryType {
            minimum: 1,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        }
    };
    let mut imports = ImportSection::new();
    imports.import(
        L::IMPORT_MODULE,
        L::IMPORT_RING_ENTER,
        EntityType::Function(void),
    );
    imports.import(
        L::IMPORT_MODULE,
        L::IMPORT_MEMORY,
        EntityType::Memory(memory),
    );
    imports.import(
        L::IMPORT_MODULE,
        L::IMPORT_TABLE,
        EntityType::Table(TableType {
            element_type: RefType::FUNCREF,
            table64: false,
            minimum: 0,
            maximum: None,
            shared: false,
        }),
    );
    if ctx.has_ref_types() {
        imports.import(
            L::IMPORT_MODULE,
            L::IMPORT_REFS,
            EntityType::Table(TableType {
                element_type: RefType::ANYREF,
                table64: false,
                minimum: 0,
                maximum: None,
                shared: false,
            }),
        );
    }
    if ctx.unwind != Unwind::Off {
        imports.import(L::IMPORT_MODULE, L::IMPORT_FRAMES, anyref_global());
    }
    if ctx.uses_spawn {
        imports.import(L::IMPORT_MODULE, L::IMPORT_SPAWN, anyref_global());
    }

    let mut module = Module::new();
    module.section(&types);
    module.section(&imports);
    module.section(&funcs);
    module.section(&exports);
    module.section(&code);
    module.section(&name_section(names));
    module.section(&version_section());
    module.finish()
}
