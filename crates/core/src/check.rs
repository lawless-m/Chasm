//! The effect checker and code generator.
//!
//! A word body is walked twice by the same [`Walker`]: a checking pass that
//! settles type variables, then an emitting pass that re-walks with the final
//! substitution and produces wasm instructions. Both passes see identical
//! input, so type-variable numbering lines up.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Instruction as I, MemArg, ValType};

use crate::ast::{Body, Lit, Node, NodeKind};
use crate::diag::{codes, Diagnostic, Location};
use crate::graph::EdgeKind;
use crate::layout;
use crate::prims;
use crate::types::{lower_all, names, Effect, Subst, Ty};

pub type WordId = usize;

/// Function indices of the runtime helpers. Index 0 is the `ring_enter` import.
pub const FN_RING_ENTER: u32 = 0;
pub const FN_ALLOC: u32 = 1;
pub const FN_TRAP: u32 = 2;
pub const FN_RING: u32 = 3;
pub const FIRST_WORD_FN: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Library,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordKind {
    Named,
    /// A quotation used as a value.
    Quote,
    /// A test thunk.
    Test,
}

#[derive(Debug, Clone)]
pub struct Compiled {
    pub locals: Vec<ValType>,
    pub code: Vec<I<'static>>,
}

#[derive(Debug, Clone)]
pub struct Word {
    pub name: String,
    pub effect: Effect,
    pub body: Option<Compiled>,
    /// The body failed to check (an error was reported).
    pub failed: bool,
    pub export: bool,
    pub origin: Origin,
    pub kind: WordKind,
    pub loc: Location,
    pub callees: Vec<(WordId, EdgeKind)>,
}

impl Word {
    pub fn func_index(id: WordId) -> u32 {
        FIRST_WORD_FN + id as u32
    }
}

/// Shared compilation state: the word database, type interner, literals.
#[derive(Default)]
pub struct Ctx {
    pub words: Vec<Word>,
    pub by_name: HashMap<String, WordId>,
    pub types: Vec<(Vec<ValType>, Vec<ValType>)>,
    type_map: HashMap<(Vec<ValType>, Vec<ValType>), u32>,
    pub literals: Vec<u8>,
    lit_map: HashMap<String, u32>,
    /// Every name defined or declared anywhere in the program, for
    /// "used before it is defined" messages.
    pub all_names: std::collections::HashSet<String>,
}

impl Ctx {
    pub fn intern_type(&mut self, params: Vec<ValType>, results: Vec<ValType>) -> u32 {
        let key = (params, results);
        if let Some(&i) = self.type_map.get(&key) {
            return i;
        }
        let i = self.types.len() as u32;
        self.types.push(key.clone());
        self.type_map.insert(key, i);
        i
    }

    /// Place a string literal in read-only data; returns (addr, len).
    pub fn intern_str(&mut self, s: &str) -> (i32, i32) {
        let len = s.len() as i32;
        if let Some(&off) = self.lit_map.get(s) {
            return ((layout::LITERALS_BASE + off) as i32, len);
        }
        let off = self.literals.len() as u32;
        self.literals.extend_from_slice(s.as_bytes());
        self.lit_map.insert(s.to_string(), off);
        ((layout::LITERALS_BASE + off) as i32, len)
    }

    pub fn add_word(&mut self, w: Word) -> WordId {
        let id = self.words.len();
        if w.kind == WordKind::Named {
            self.by_name.insert(w.name.clone(), id);
        }
        self.words.push(w);
        id
    }
}

/// What to compile: a named word with a declared effect, or a body whose
/// effect is derived by forward checking from an empty stack.
pub enum Mode<'a> {
    Declared(&'a Effect),
    Derived,
}

pub struct Output {
    pub effect: Effect,
    pub compiled: Compiled,
    pub callees: Vec<(WordId, EdgeKind)>,
}

/// Compile one body. `outer_locals` are names visible in an enclosing word,
/// used only to report captures.
pub fn compile_body(
    ctx: &mut Ctx,
    name: &str,
    mode: Mode<'_>,
    body: &Body,
    loc: &Location,
    outer_locals: &[String],
) -> Result<Output, Diagnostic> {
    let mut first = Walker::new(ctx, name, false, Subst::default(), outer_locals);
    let effect = first.run(&mode, body, loc)?;
    let mut subst = first.subst;
    subst.restart();
    let mut second = Walker::new(ctx, name, true, subst, outer_locals);
    let effect2 = second.run(&mode, body, loc)?;
    debug_assert_eq!(effect, effect2);
    let compiled = Compiled {
        locals: second.local_types,
        code: second.code,
    };
    let mut callees = second.callees;
    callees.sort();
    callees.dedup();
    Ok(Output {
        effect,
        compiled,
        callees,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Normal,
    Diverged,
}

struct Local {
    name: String,
    ty: Ty,
    mutable: bool,
    idx: Vec<u32>,
}

struct LoopCtx {
    /// `None` for map/filter, where `leave` is not allowed.
    exit: Option<Vec<Ty>>,
    /// Label level of the block that `leave` branches to.
    level: u32,
}

#[derive(Default)]
struct TempAlloc {
    counts: [u32; 4],
}

fn vt_key(vt: ValType) -> usize {
    match vt {
        ValType::I32 => 0,
        ValType::I64 => 1,
        ValType::F32 => 2,
        _ => 3,
    }
}

fn vt_size(vt: ValType) -> u32 {
    match vt {
        ValType::I64 | ValType::F64 => 8,
        _ => 4,
    }
}

fn memarg(offset: u32, vt: ValType) -> MemArg {
    MemArg {
        offset: offset as u64,
        align: if vt_size(vt) == 8 { 3 } else { 2 },
        memory_index: 0,
    }
}

fn load_vt(vt: ValType, offset: u32) -> I<'static> {
    let m = memarg(offset, vt);
    match vt {
        ValType::I64 => I::I64Load(m),
        ValType::F32 => I::F32Load(m),
        ValType::F64 => I::F64Load(m),
        _ => I::I32Load(m),
    }
}

fn store_vt(vt: ValType, offset: u32) -> I<'static> {
    let m = memarg(offset, vt);
    match vt {
        ValType::I64 => I::I64Store(m),
        ValType::F32 => I::F32Store(m),
        ValType::F64 => I::F64Store(m),
        _ => I::I32Store(m),
    }
}

fn fmt_stack(tys: &[Ty]) -> String {
    if tys.is_empty() {
        "( )".to_string()
    } else {
        format!("( {} )", names(tys).join(" "))
    }
}

struct Walker<'c> {
    ctx: &'c mut Ctx,
    name: String,
    emit: bool,
    subst: Subst,
    stack: Vec<Ty>,
    locals: Vec<Local>,
    outer_locals: Vec<String>,
    nparams: u32,
    local_types: Vec<ValType>,
    temps: HashMap<(usize, u32), u32>,
    code: Vec<I<'static>>,
    depth: u32,
    loops: Vec<LoopCtx>,
    callees: Vec<(WordId, EdgeKind)>,
}

impl<'c> Walker<'c> {
    fn new(
        ctx: &'c mut Ctx,
        name: &str,
        emit: bool,
        subst: Subst,
        outer_locals: &[String],
    ) -> Self {
        Walker {
            ctx,
            name: name.to_string(),
            emit,
            subst,
            stack: Vec::new(),
            locals: Vec::new(),
            outer_locals: outer_locals.to_vec(),
            nparams: 0,
            local_types: Vec::new(),
            temps: HashMap::new(),
            code: Vec::new(),
            depth: 0,
            loops: Vec::new(),
            callees: Vec::new(),
        }
    }

    fn run(&mut self, mode: &Mode<'_>, body: &Body, loc: &Location) -> Result<Effect, Diagnostic> {
        if let Mode::Declared(e) = mode {
            self.stack = e.inputs.clone();
            let params = e.wasm_params();
            self.nparams = params.len() as u32;
            for i in 0..self.nparams {
                self.op(I::LocalGet(i));
            }
        }
        let flow = self.seq(body)?;
        match mode {
            Mode::Declared(e) => {
                if flow == Flow::Normal && !self.unify_stack(&e.outputs) {
                    let actual = self.resolved_stack();
                    return Err(Diagnostic::error(
                        codes::E_EFFECT_MISMATCH,
                        format!(
                            "`{}` is declared {} but its body leaves {}",
                            self.name,
                            e,
                            fmt_stack(&actual)
                        ),
                        loc.clone(),
                    )
                    .with_word(&self.name)
                    .with_stacks(names(&e.outputs), names(&actual)));
                }
                Ok((*e).clone())
            }
            Mode::Derived => {
                let outputs = if flow == Flow::Normal {
                    self.resolved_stack()
                } else {
                    Vec::new()
                };
                if let Some(t) = outputs.iter().find(|t| t.has_var()) {
                    return Err(Diagnostic::error(
                        codes::E_AMBIGUOUS_TYPE,
                        format!("the type `{t}` left by this code is not fully known; add a stack assertion"),
                        loc.clone(),
                    ));
                }
                Ok(Effect::new(Vec::new(), outputs))
            }
        }
    }

    // ---- small helpers ----

    fn op(&mut self, i: I<'static>) {
        self.code.push(i);
    }

    fn resolved_stack(&self) -> Vec<Ty> {
        self.subst.resolve_all(&self.stack)
    }

    fn unify_stack(&mut self, want: &[Ty]) -> bool {
        let stack = self.stack.clone();
        self.subst.unify_all(&stack, want)
    }

    fn lower(&self, t: &Ty) -> Vec<ValType> {
        self.subst.resolve(t).lower()
    }

    fn lower_all(&self, tys: &[Ty]) -> Vec<ValType> {
        lower_all(&self.subst.resolve_all(tys))
    }

    fn block_type(&mut self, params: &[Ty], results: &[Ty]) -> BlockType {
        if !self.emit {
            return BlockType::Empty;
        }
        let p = self.lower_all(params);
        let r = self.lower_all(results);
        if p.is_empty() && r.is_empty() {
            return BlockType::Empty;
        }
        if p.is_empty() && r.len() == 1 {
            return BlockType::Result(r[0]);
        }
        BlockType::FunctionType(self.ctx.intern_type(p, r))
    }

    fn new_local(&mut self, vt: ValType) -> u32 {
        let idx = self.nparams + self.local_types.len() as u32;
        self.local_types.push(vt);
        idx
    }

    fn temp(&mut self, ta: &mut TempAlloc, vt: ValType) -> u32 {
        let k = vt_key(vt);
        let n = ta.counts[k];
        ta.counts[k] += 1;
        if let Some(&i) = self.temps.get(&(k, n)) {
            return i;
        }
        let i = self.new_local(vt);
        self.temps.insert((k, n), i);
        i
    }

    /// Pop `tys` (bottom..top) from the wasm stack into locals.
    /// `ta = None` allocates fresh locals that stay live across nested code.
    fn stash(&mut self, tys: &[Ty], mut ta: Option<&mut TempAlloc>) -> Vec<Vec<u32>> {
        let mut out: Vec<Vec<u32>> = Vec::new();
        for t in tys {
            let mut v = Vec::new();
            for vt in self.lower(t) {
                let i = match ta.as_deref_mut() {
                    Some(ta) => self.temp(ta, vt),
                    None => self.new_local(vt),
                };
                v.push(i);
            }
            out.push(v);
        }
        for v in out.iter().rev() {
            for &i in v.iter().rev() {
                self.op(I::LocalSet(i));
            }
        }
        out
    }

    fn unstash(&mut self, idx: &[u32]) {
        for &i in idx {
            self.op(I::LocalGet(i));
        }
    }

    fn trap(&mut self, msg: &str) {
        let (ma, ml) = self.ctx.intern_str(msg);
        let (wa, wl) = self.ctx.intern_str(&self.name.clone());
        self.op(I::I32Const(ma));
        self.op(I::I32Const(ml));
        self.op(I::I32Const(wa));
        self.op(I::I32Const(wl));
        self.op(I::Call(FN_TRAP));
        self.op(I::Unreachable);
    }

    fn err(&self, code: &str, msg: impl Into<String>, loc: &Location) -> Diagnostic {
        Diagnostic::error(code, msg, loc.clone()).with_word(&self.name)
    }

    /// Pop values matching `ins` (bottom..top).
    fn pop_expect(&mut self, what: &str, ins: &[Ty], loc: &Location) -> Result<(), Diagnostic> {
        let n = ins.len();
        if self.stack.len() < n {
            let actual = self.resolved_stack();
            return Err(self
                .err(
                    codes::E_STACK_UNDERFLOW,
                    format!(
                        "`{what}` needs {} but the stack is {}",
                        fmt_stack(ins),
                        fmt_stack(&actual)
                    ),
                    loc,
                )
                .with_stacks(names(ins), names(&actual)));
        }
        let at = self.stack.len() - n;
        let top: Vec<Ty> = self.stack[at..].to_vec();
        if !self.subst.unify_all(&top, ins) {
            let top = self.subst.resolve_all(&top);
            let want = self.subst.resolve_all(ins);
            return Err(self
                .err(
                    codes::E_TYPE_MISMATCH,
                    format!(
                        "`{what}` needs {} on top of the stack but found {}",
                        fmt_stack(&want),
                        fmt_stack(&top)
                    ),
                    loc,
                )
                .with_stacks(names(&want), names(&top)));
        }
        self.stack.truncate(at);
        Ok(())
    }

    fn pop_any(&mut self, what: &str, n: usize, loc: &Location) -> Result<Vec<Ty>, Diagnostic> {
        if self.stack.len() < n {
            let actual = self.resolved_stack();
            let want: Vec<String> = (0..n)
                .map(|i| format!("{}", (b'a' + i as u8) as char))
                .collect();
            return Err(self
                .err(
                    codes::E_STACK_UNDERFLOW,
                    format!(
                        "`{what}` needs {n} value{} but the stack is {}",
                        if n == 1 { "" } else { "s" },
                        fmt_stack(&actual)
                    ),
                    loc,
                )
                .with_stacks(want, names(&actual)));
        }
        let at = self.stack.len() - n;
        Ok(self.stack.split_off(at))
    }

    fn pop_array(&mut self, what: &str, loc: &Location) -> Result<Ty, Diagnostic> {
        let v = self.subst.fresh();
        self.pop_expect(what, &[Ty::Array(Box::new(v.clone()))], loc)?;
        Ok(self.subst.resolve(&v))
    }

    /// In the emitting pass, an element type must be known.
    fn concrete_elem(&self, t: &Ty, what: &str, loc: &Location) -> Result<Ty, Diagnostic> {
        let t = self.subst.resolve(t);
        if self.emit && t.has_var() {
            return Err(self.err(
                codes::E_AMBIGUOUS_TYPE,
                format!(
                    "the element type of `{what}` is not known; fix it with a stack assertion such as ( array i32 ) or by binding the array to a local that is used as one"
                ),
                loc,
            ));
        }
        if matches!(t, Ty::Array(_)) {
            return Err(self.err(
                codes::E_TYPE_MISMATCH,
                "nested arrays are not supported in v1",
                loc,
            ));
        }
        Ok(t)
    }

    fn open_label(&mut self) -> u32 {
        self.depth += 1;
        self.depth
    }

    fn close_label(&mut self) {
        self.op(I::End);
        self.depth -= 1;
    }

    fn rel(&self, level: u32) -> u32 {
        self.depth - level
    }

    /// Walk a sequence into a separate code buffer.
    fn seq_into(&mut self, body: &Body) -> Result<(Flow, Vec<I<'static>>), Diagnostic> {
        let saved = std::mem::take(&mut self.code);
        let r = self.seq(body);
        let code = std::mem::replace(&mut self.code, saved);
        Ok((r?, code))
    }

    fn seq(&mut self, body: &Body) -> Result<Flow, Diagnostic> {
        let mut flow = Flow::Normal;
        for node in body {
            if flow == Flow::Diverged {
                return Err(self.err(
                    codes::E_UNREACHABLE,
                    "code after `leave` or `trap` can never run",
                    &node.loc,
                ));
            }
            flow = self.node(node)?;
        }
        Ok(flow)
    }

    fn node(&mut self, node: &Node) -> Result<Flow, Diagnostic> {
        let loc = &node.loc;
        match &node.kind {
            NodeKind::Lit(l) => {
                match l {
                    Lit::I32(v) => self.op(I::I32Const(*v)),
                    Lit::I64(v) => self.op(I::I64Const(*v)),
                    Lit::F32(v) => self.op(I::F32Const((*v).into())),
                    Lit::F64(v) => self.op(I::F64Const((*v).into())),
                    Lit::Str(s) => {
                        let (a, n) = self.ctx.intern_str(s);
                        self.op(I::I32Const(a));
                        self.op(I::I32Const(n));
                    }
                }
                self.stack.push(l.ty());
                Ok(Flow::Normal)
            }
            NodeKind::Name(n) => self.name_ref(n, loc),
            NodeKind::Bind { name, mutable } => {
                if self.locals.iter().any(|l| &l.name == name) {
                    return Err(self.err(
                        codes::E_LOCAL,
                        format!("local `{name}` is already bound in this word"),
                        loc,
                    ));
                }
                if prims::is_builtin(name) {
                    return Err(self.err(
                        codes::E_LOCAL,
                        format!("`{name}` is a primitive and cannot be used as a local name"),
                        loc,
                    ));
                }
                let mut tys = self.pop_any(&format!(":> {name}"), 1, loc)?;
                let ty = tys.pop().unwrap();
                let idx = self.stash(std::slice::from_ref(&ty), None).pop().unwrap();
                self.locals.push(Local {
                    name: name.clone(),
                    ty,
                    mutable: *mutable,
                    idx,
                });
                Ok(Flow::Normal)
            }
            NodeKind::Tick(n) => {
                let Some(&id) = self.ctx.by_name.get(n) else {
                    return Err(self.undefined(n, loc));
                };
                let e = self.ctx.words[id].effect.clone();
                self.callees.push((id, EdgeKind::AddressTaken));
                self.op(I::I32Const(id as i32));
                self.stack.push(Ty::Quot(Box::new(e)));
                Ok(Flow::Normal)
            }
            NodeKind::Quote(body) => {
                let mut visible: Vec<String> = self.outer_locals.clone();
                visible.extend(self.locals.iter().map(|l| l.name.clone()));
                let qname = format!("[quote {}:{}:{}]", self.name, loc.line, loc.column);
                let out = compile_body(self.ctx, &qname, Mode::Derived, body, loc, &visible)?;
                let ty = Ty::Quot(Box::new(out.effect.clone()));
                if self.emit {
                    let id = self.ctx.add_word(Word {
                        name: qname,
                        effect: out.effect,
                        body: Some(out.compiled),
                        failed: false,
                        export: false,
                        origin: Origin::User,
                        kind: WordKind::Quote,
                        loc: loc.clone(),
                        callees: out.callees,
                    });
                    self.callees.push((id, EdgeKind::AddressTaken));
                    self.op(I::I32Const(id as i32));
                } else {
                    self.op(I::I32Const(0));
                }
                self.stack.push(ty);
                Ok(Flow::Normal)
            }
            NodeKind::Assert(tys) => {
                if !self.unify_stack(tys) {
                    let actual = self.resolved_stack();
                    return Err(self
                        .err(
                            codes::E_ASSERTION,
                            format!(
                                "stack assertion {} does not hold; the stack is {}",
                                fmt_stack(tys),
                                fmt_stack(&actual)
                            ),
                            loc,
                        )
                        .with_stacks(names(tys), names(&actual)));
                }
                Ok(Flow::Normal)
            }
            NodeKind::If(t, e) => {
                self.pop_expect("if", &[Ty::I32], loc)?;
                let s = self.stack.clone();
                self.open_label();
                let (ft, ct) = self.seq_into(t)?;
                let st = std::mem::replace(&mut self.stack, s.clone());
                let (fe, ce) = self.seq_into(e)?;
                let se = std::mem::replace(&mut self.stack, s.clone());
                self.depth -= 1;
                let (flow, result) = match (ft, fe) {
                    (Flow::Normal, Flow::Normal) => {
                        if !self.subst.unify_all(&st, &se) {
                            let (a, b) = (self.subst.resolve_all(&st), self.subst.resolve_all(&se));
                            return Err(self
                                .err(
                                    codes::E_BRANCH_MISMATCH,
                                    format!(
                                        "branches of `if` disagree: then-branch leaves {}, else-branch leaves {}",
                                        fmt_stack(&a),
                                        fmt_stack(&b)
                                    ),
                                    loc,
                                )
                                .with_stacks(names(&a), names(&b)));
                        }
                        (Flow::Normal, st)
                    }
                    (Flow::Normal, Flow::Diverged) => (Flow::Normal, st),
                    (Flow::Diverged, Flow::Normal) => (Flow::Normal, se),
                    (Flow::Diverged, Flow::Diverged) => (Flow::Diverged, s.clone()),
                };
                let bt = self.block_type(&s, &result);
                self.op(I::If(bt));
                self.code.extend(ct);
                self.op(I::Else);
                self.code.extend(ce);
                self.op(I::End);
                self.stack = result;
                Ok(flow)
            }
            NodeKind::When(b) | NodeKind::Unless(b) => {
                let what = if matches!(node.kind, NodeKind::When(_)) {
                    "when"
                } else {
                    "unless"
                };
                self.pop_expect(what, &[Ty::I32], loc)?;
                let s = self.stack.clone();
                if what == "unless" {
                    self.op(I::I32Eqz);
                }
                let bt = self.block_type(&s, &s);
                self.op(I::If(bt));
                self.open_label();
                let flow = self.seq(b)?;
                self.expect_identity(flow, &s, what, "body", loc)?;
                self.close_label();
                self.stack = s;
                Ok(Flow::Normal)
            }
            NodeKind::While(c, b) => {
                let s = self.stack.clone();
                let bt = self.block_type(&s, &s);
                self.op(I::Block(bt));
                let exit = self.open_label();
                self.op(I::Loop(bt));
                let top = self.open_label();
                self.loops.push(LoopCtx {
                    exit: Some(s.clone()),
                    level: exit,
                });
                let fc = self.seq(c)?;
                if fc == Flow::Normal {
                    let mut want = s.clone();
                    want.push(Ty::I32);
                    self.expect_shape(&want, "while", "condition", "plus one i32", loc)?;
                    self.stack.pop();
                }
                self.op(I::I32Eqz);
                self.op(I::BrIf(self.rel(exit)));
                let fb = self.seq(b)?;
                self.expect_identity(fb, &s, "while", "body", loc)?;
                self.op(I::Br(self.rel(top)));
                self.loops.pop();
                self.close_label();
                self.close_label();
                self.stack = s;
                Ok(Flow::Normal)
            }
            NodeKind::Until(b, c) => {
                let s = self.stack.clone();
                let bt = self.block_type(&s, &s);
                self.op(I::Block(bt));
                let exit = self.open_label();
                self.op(I::Loop(bt));
                let top = self.open_label();
                self.loops.push(LoopCtx {
                    exit: Some(s.clone()),
                    level: exit,
                });
                let fb = self.seq(b)?;
                self.expect_identity(fb, &s, "until", "body", loc)?;
                let fc = self.seq(c)?;
                if fc == Flow::Normal {
                    let mut want = s.clone();
                    want.push(Ty::I32);
                    self.expect_shape(&want, "until", "condition", "plus one i32", loc)?;
                    self.stack.pop();
                }
                self.op(I::I32Eqz);
                self.op(I::BrIf(self.rel(top)));
                self.loops.pop();
                self.close_label();
                self.close_label();
                self.stack = s;
                Ok(Flow::Normal)
            }
            NodeKind::Times(b) => {
                self.pop_expect("times", &[Ty::I32], loc)?;
                let s = self.stack.clone();
                let n = self.new_local(ValType::I32);
                let i = self.new_local(ValType::I32);
                self.op(I::LocalSet(n));
                self.op(I::I32Const(0));
                self.op(I::LocalSet(i));
                let bt = self.block_type(&s, &s);
                self.op(I::Block(bt));
                let exit = self.open_label();
                self.op(I::Loop(bt));
                let top = self.open_label();
                self.op(I::LocalGet(i));
                self.op(I::LocalGet(n));
                self.op(I::I32GeS);
                self.op(I::BrIf(self.rel(exit)));
                self.op(I::LocalGet(i));
                self.stack.push(Ty::I32);
                self.loops.push(LoopCtx {
                    exit: Some(s.clone()),
                    level: exit,
                });
                let fb = self.seq(b)?;
                self.expect_identity(fb, &s, "times", "body (which receives the index)", loc)?;
                self.loops.pop();
                self.op(I::LocalGet(i));
                self.op(I::I32Const(1));
                self.op(I::I32Add);
                self.op(I::LocalSet(i));
                self.op(I::Br(self.rel(top)));
                self.close_label();
                self.close_label();
                self.stack = s;
                Ok(Flow::Normal)
            }
            NodeKind::Leave => {
                let Some(lp) = self.loops.last() else {
                    return Err(self.err(
                        codes::E_LEAVE,
                        "`leave` is only allowed inside a loop (while, until, times, each, fold)",
                        loc,
                    ));
                };
                let Some(exit) = lp.exit.clone() else {
                    return Err(self.err(
                        codes::E_LEAVE,
                        "`leave` is not allowed inside map or filter",
                        loc,
                    ));
                };
                let level = lp.level;
                if !self.unify_stack(&exit) {
                    let (want, actual) = (self.subst.resolve_all(&exit), self.resolved_stack());
                    return Err(self
                        .err(
                            codes::E_LEAVE,
                            format!(
                                "the stack at `leave` must match the loop's exit shape {} but is {}",
                                fmt_stack(&want),
                                fmt_stack(&actual)
                            ),
                            loc,
                        )
                        .with_stacks(names(&want), names(&actual)));
                }
                self.op(I::Br(self.rel(level)));
                Ok(Flow::Diverged)
            }
            NodeKind::Each(b) => self.each(b, loc),
            NodeKind::Map(b) => self.map(b, loc),
            NodeKind::Filter(b) => self.filter(b, loc),
            NodeKind::Fold(b) => self.fold(b, loc),
        }
    }

    fn expect_shape(
        &mut self,
        want: &[Ty],
        comb: &str,
        part: &str,
        extra: &str,
        loc: &Location,
    ) -> Result<(), Diagnostic> {
        if self.unify_stack(want) {
            return Ok(());
        }
        let (w, a) = (self.subst.resolve_all(want), self.resolved_stack());
        Err(self
            .err(
                codes::E_LOOP_EFFECT,
                format!(
                    "the {part} of `{comb}` must leave the stack as it found it{}{}: expected {}, found {}",
                    if extra.is_empty() { "" } else { " " },
                    extra,
                    fmt_stack(&w),
                    fmt_stack(&a)
                ),
                loc,
            )
            .with_stacks(names(&w), names(&a)))
    }

    fn expect_identity(
        &mut self,
        flow: Flow,
        s: &[Ty],
        comb: &str,
        part: &str,
        loc: &Location,
    ) -> Result<(), Diagnostic> {
        if flow == Flow::Diverged {
            return Ok(());
        }
        let code = if comb == "when" || comb == "unless" {
            codes::E_BRANCH_MISMATCH
        } else {
            codes::E_LOOP_EFFECT
        };
        if self.unify_stack(s) {
            return Ok(());
        }
        let (w, a) = (self.subst.resolve_all(s), self.resolved_stack());
        Err(self
            .err(
                code,
                format!(
                    "the {part} of `{comb}` must leave the stack unchanged: expected {}, found {}",
                    fmt_stack(&w),
                    fmt_stack(&a)
                ),
                loc,
            )
            .with_stacks(names(&w), names(&a)))
    }

    /// Emit: push element `i` of the array at `addr` (locals) onto the stack.
    fn load_elem(&mut self, t: &Ty, addr: u32, i: u32, ta: &mut TempAlloc) {
        let size = t.elem_size() as i32;
        let ea = self.temp(ta, ValType::I32);
        self.op(I::LocalGet(addr));
        self.op(I::LocalGet(i));
        self.op(I::I32Const(size));
        self.op(I::I32Mul);
        self.op(I::I32Add);
        self.op(I::LocalSet(ea));
        let mut off = 0;
        for vt in t.lower() {
            self.op(I::LocalGet(ea));
            self.op(load_vt(vt, off));
            off += vt_size(vt);
        }
    }

    /// Emit: store value locals `val` as element `i` of the array at `addr`.
    fn store_elem(&mut self, t: &Ty, addr: u32, i: u32, val: &[u32], ta: &mut TempAlloc) {
        let size = t.elem_size() as i32;
        let ea = self.temp(ta, ValType::I32);
        self.op(I::LocalGet(addr));
        self.op(I::LocalGet(i));
        self.op(I::I32Const(size));
        self.op(I::I32Mul);
        self.op(I::I32Add);
        self.op(I::LocalSet(ea));
        let mut off = 0;
        for (vt, &v) in t.lower().into_iter().zip(val) {
            self.op(I::LocalGet(ea));
            self.op(I::LocalGet(v));
            self.op(store_vt(vt, off));
            off += vt_size(vt);
        }
    }

    /// Common loop head for collection combinators. Returns (exit level, top level).
    fn coll_loop_open(&mut self, s: &[Ty], i: u32, len: u32) -> (u32, u32) {
        let bt = self.block_type(s, s);
        self.op(I::I32Const(0));
        self.op(I::LocalSet(i));
        self.op(I::Block(bt));
        let exit = self.open_label();
        self.op(I::Loop(bt));
        let top = self.open_label();
        self.op(I::LocalGet(i));
        self.op(I::LocalGet(len));
        self.op(I::I32GeU);
        self.op(I::BrIf(self.rel(exit)));
        (exit, top)
    }

    fn coll_loop_close(&mut self, i: u32, top: u32) {
        self.op(I::LocalGet(i));
        self.op(I::I32Const(1));
        self.op(I::I32Add);
        self.op(I::LocalSet(i));
        self.op(I::Br(self.rel(top)));
        self.close_label();
        self.close_label();
    }

    fn each(&mut self, b: &Body, loc: &Location) -> Result<Flow, Diagnostic> {
        let t = self.pop_array("each", loc)?;
        let t = self.concrete_elem(&t, "each", loc)?;
        let s = self.stack.clone();
        let len = self.new_local(ValType::I32);
        let addr = self.new_local(ValType::I32);
        let i = self.new_local(ValType::I32);
        self.op(I::LocalSet(len));
        self.op(I::LocalSet(addr));
        let (exit, top) = self.coll_loop_open(&s, i, len);
        self.load_elem(&t, addr, i, &mut TempAlloc::default());
        self.stack.push(t);
        self.loops.push(LoopCtx {
            exit: Some(s.clone()),
            level: exit,
        });
        let f = self.seq(b)?;
        self.expect_identity(f, &s, "each", "body (which receives each element)", loc)?;
        self.loops.pop();
        self.coll_loop_close(i, top);
        self.stack = s;
        Ok(Flow::Normal)
    }

    fn map(&mut self, b: &Body, loc: &Location) -> Result<Flow, Diagnostic> {
        let t = self.pop_array("map", loc)?;
        let t = self.concrete_elem(&t, "map", loc)?;
        let s = self.stack.clone();
        let len = self.new_local(ValType::I32);
        let src = self.new_local(ValType::I32);
        let dst = self.new_local(ValType::I32);
        let i = self.new_local(ValType::I32);
        // Walk the body first (into its own buffer) to learn U.
        self.depth += 2;
        self.loops.push(LoopCtx {
            exit: None,
            level: 0,
        });
        self.stack.push(t.clone());
        let saved = std::mem::take(&mut self.code);
        let mut ta = TempAlloc::default();
        self.load_elem(&t, src, i, &mut ta);
        let f = self.seq(b);
        let body_code = std::mem::replace(&mut self.code, saved);
        let f = f?;
        self.loops.pop();
        self.depth -= 2;
        if f == Flow::Diverged {
            return Err(self.err(
                codes::E_LOOP_EFFECT,
                "the body of `map` must produce a value",
                loc,
            ));
        }
        if self.stack.len() != s.len() + 1 || !self.subst.unify_all(&self.stack[..s.len()], &s) {
            let actual = self.resolved_stack();
            let mut want = names(&s);
            want.push("U".into());
            return Err(self
                .err(
                    codes::E_LOOP_EFFECT,
                    format!(
                        "the body of `map` must replace the element with exactly one value: expected ( {} ), found {}",
                        want.join(" "),
                        fmt_stack(&actual)
                    ),
                    loc,
                )
                .with_stacks(want, names(&actual)));
        }
        let u = self.stack.pop().unwrap();
        let u = self.concrete_elem(&u, "map", loc)?;
        // Prologue.
        self.op(I::LocalSet(len));
        self.op(I::LocalSet(src));
        self.op(I::LocalGet(len));
        self.op(I::I32Const(u.elem_size() as i32));
        self.op(I::I32Mul);
        self.op(I::Call(FN_ALLOC));
        self.op(I::LocalSet(dst));
        let (_exit, top) = self.coll_loop_open(&s, i, len);
        self.code.extend(body_code);
        let val = self.stash(std::slice::from_ref(&u), None).pop().unwrap();
        self.store_elem(&u, dst, i, &val, &mut TempAlloc::default());
        self.coll_loop_close(i, top);
        self.op(I::LocalGet(dst));
        self.op(I::LocalGet(len));
        self.stack = s;
        self.stack.push(Ty::Array(Box::new(u)));
        Ok(Flow::Normal)
    }

    fn filter(&mut self, b: &Body, loc: &Location) -> Result<Flow, Diagnostic> {
        let t = self.pop_array("filter", loc)?;
        let t = self.concrete_elem(&t, "filter", loc)?;
        let s = self.stack.clone();
        let len = self.new_local(ValType::I32);
        let src = self.new_local(ValType::I32);
        let dst = self.new_local(ValType::I32);
        let i = self.new_local(ValType::I32);
        let cnt = self.new_local(ValType::I32);
        self.op(I::LocalSet(len));
        self.op(I::LocalSet(src));
        self.op(I::LocalGet(len));
        self.op(I::I32Const(t.elem_size() as i32));
        self.op(I::I32Mul);
        self.op(I::Call(FN_ALLOC));
        self.op(I::LocalSet(dst));
        self.op(I::I32Const(0));
        self.op(I::LocalSet(cnt));
        let (_exit, top) = self.coll_loop_open(&s, i, len);
        self.load_elem(&t, src, i, &mut TempAlloc::default());
        let val = self.stash(std::slice::from_ref(&t), None).pop().unwrap();
        self.unstash(&val);
        self.stack.push(t.clone());
        self.loops.push(LoopCtx {
            exit: None,
            level: 0,
        });
        let f = self.seq(b)?;
        self.loops.pop();
        if f == Flow::Normal {
            let mut want = s.clone();
            want.push(Ty::I32);
            self.expect_shape(&want, "filter", "predicate", "plus one i32", loc)?;
            self.stack.pop();
        }
        self.op(I::If(BlockType::Empty));
        self.depth += 1;
        self.store_elem(&t, dst, cnt, &val, &mut TempAlloc::default());
        self.op(I::LocalGet(cnt));
        self.op(I::I32Const(1));
        self.op(I::I32Add);
        self.op(I::LocalSet(cnt));
        self.close_label();
        self.coll_loop_close(i, top);
        self.op(I::LocalGet(dst));
        self.op(I::LocalGet(cnt));
        self.stack = s;
        self.stack.push(Ty::Array(Box::new(t)));
        Ok(Flow::Normal)
    }

    fn fold(&mut self, b: &Body, loc: &Location) -> Result<Flow, Diagnostic> {
        let mut init = self.pop_any("fold", 1, loc)?;
        let u = init.pop().unwrap();
        let t = self.pop_array("fold", loc)?;
        let t = self.concrete_elem(&t, "fold", loc)?;
        let s = self.stack.clone();
        let mut su = s.clone();
        su.push(u.clone());
        let mut ta = TempAlloc::default();
        let acc = self
            .stash(std::slice::from_ref(&u), Some(&mut ta))
            .pop()
            .unwrap();
        let len = self.new_local(ValType::I32);
        let src = self.new_local(ValType::I32);
        let i = self.new_local(ValType::I32);
        self.op(I::LocalSet(len));
        self.op(I::LocalSet(src));
        self.unstash(&acc);
        let (exit, top) = self.coll_loop_open(&su, i, len);
        self.load_elem(&t, src, i, &mut TempAlloc::default());
        self.stack = su.clone();
        self.stack.push(t);
        self.loops.push(LoopCtx {
            exit: Some(su.clone()),
            level: exit,
        });
        let f = self.seq(b)?;
        if f == Flow::Normal {
            self.expect_shape(&su, "fold", "body", "with the accumulator replaced", loc)?;
        }
        self.loops.pop();
        self.coll_loop_close(i, top);
        self.stack = su;
        Ok(Flow::Normal)
    }

    fn undefined(&self, n: &str, loc: &Location) -> Diagnostic {
        let msg = if self.outer_locals.iter().any(|l| l == n) {
            return self.err(
                codes::E_CAPTURE,
                format!("quotation values cannot use local `{n}` of the enclosing word (closures are not v1); pass it on the stack or use a named word"),
                loc,
            );
        } else if self.ctx.all_names.contains(n) {
            format!("`{n}` is used before it is defined; add `declare {n} ( ... -- ... )` above this use")
        } else {
            format!("unknown word `{n}`")
        };
        self.err(codes::E_UNDEFINED, msg, loc)
    }

    fn name_ref(&mut self, n: &str, loc: &Location) -> Result<Flow, Diagnostic> {
        // Locals.
        if let Some(l) = self.locals.iter().find(|l| l.name == n) {
            let (ty, idx) = (l.ty.clone(), l.idx.clone());
            self.unstash(&idx);
            self.stack.push(ty);
            return Ok(Flow::Normal);
        }
        if let Some(base) = n.strip_suffix('!') {
            if let Some(l) = self.locals.iter().find(|l| l.name == base) {
                if !l.mutable {
                    return Err(self.err(
                        codes::E_LOCAL,
                        format!("local `{base}` is immutable; bind it with `:> {base}!` to allow assignment"),
                        loc,
                    ));
                }
                let (ty, idx) = (l.ty.clone(), l.idx.clone());
                self.pop_expect(n, std::slice::from_ref(&ty), loc)?;
                for &i in idx.iter().rev() {
                    self.op(I::LocalSet(i));
                }
                return Ok(Flow::Normal);
            }
        }
        // Shuffles.
        if let Some((k, perm)) = prims::shuffle(n) {
            let tys = self.pop_any(n, k, loc)?;
            let tys = self.subst.resolve_all(&tys);
            if perm.is_empty() {
                for _ in self.lower_all(&tys) {
                    self.op(I::Drop);
                }
            } else {
                let mut ta = TempAlloc::default();
                let idx = self.stash(&tys, Some(&mut ta));
                for &p in perm {
                    self.unstash(&idx[p]);
                }
            }
            for &p in perm {
                self.stack.push(tys[p].clone());
            }
            return Ok(Flow::Normal);
        }
        // Numeric and memory instructions.
        if let Some((ins, outs, instr)) = prims::numeric(n) {
            self.pop_expect(n, &ins, loc)?;
            self.op(instr);
            self.stack.extend(outs);
            return Ok(Flow::Normal);
        }
        if let Some((ins, outs)) = prims::special(n) {
            self.pop_expect(n, &ins, loc)?;
            match n {
                "str.len" => {
                    let mut ta = TempAlloc::default();
                    let t = self.temp(&mut ta, ValType::I32);
                    self.op(I::LocalSet(t));
                    self.op(I::Drop);
                    self.op(I::LocalGet(t));
                }
                "str.addr" => self.op(I::Drop),
                "str.from-raw" => {}
                "mem.alloc" => self.op(I::Call(FN_ALLOC)),
                "trap" => {
                    let (wa, wl) = self.ctx.intern_str(&self.name.clone());
                    self.op(I::I32Const(wa));
                    self.op(I::I32Const(wl));
                    self.op(I::Call(FN_TRAP));
                    self.op(I::Unreachable);
                    return Ok(Flow::Diverged);
                }
                "host.open" => {
                    self.op(I::I32Const(layout::OP_OPEN));
                    self.op(I::Call(FN_RING));
                }
                "host.read" | "host.write" => {
                    let op = if n == "host.read" {
                        layout::OP_READ
                    } else {
                        layout::OP_WRITE
                    };
                    self.op(I::I32Const(op));
                    self.op(I::Call(FN_RING));
                }
                "host.close" => {
                    self.op(I::I32Const(0));
                    self.op(I::I32Const(0));
                    self.op(I::I32Const(layout::OP_CLOSE));
                    self.op(I::Call(FN_RING));
                }
                _ => unreachable!(),
            }
            self.stack.extend(outs);
            return Ok(Flow::Normal);
        }
        match n {
            "array.new" => {
                self.pop_expect(n, &[Ty::I32], loc)?;
                let v = self.subst.fresh();
                let t = self.concrete_elem(&v, n, loc)?;
                let mut ta = TempAlloc::default();
                let c = self.temp(&mut ta, ValType::I32);
                self.op(I::LocalTee(c));
                self.op(I::I32Const(t.elem_size() as i32));
                self.op(I::I32Mul);
                self.op(I::Call(FN_ALLOC));
                self.op(I::LocalGet(c));
                self.stack.push(Ty::Array(Box::new(v)));
                return Ok(Flow::Normal);
            }
            "array.len" => {
                self.pop_array(n, loc)?;
                let mut ta = TempAlloc::default();
                let t = self.temp(&mut ta, ValType::I32);
                self.op(I::LocalSet(t));
                self.op(I::Drop);
                self.op(I::LocalGet(t));
                self.stack.push(Ty::I32);
                return Ok(Flow::Normal);
            }
            "array.at" | "array.at!" => {
                let v = self.subst.fresh();
                let mut ins = vec![Ty::Array(Box::new(v.clone())), Ty::I32];
                let store = n == "array.at!";
                if store {
                    ins.push(v.clone());
                }
                self.pop_expect(n, &ins, loc)?;
                let t = self.concrete_elem(&v, n, loc)?;
                let mut ta = TempAlloc::default();
                let val = if store {
                    self.stash(std::slice::from_ref(&t), Some(&mut ta))
                        .pop()
                        .unwrap()
                } else {
                    Vec::new()
                };
                let idx = self.temp(&mut ta, ValType::I32);
                let len = self.temp(&mut ta, ValType::I32);
                let addr = self.temp(&mut ta, ValType::I32);
                self.op(I::LocalSet(idx));
                self.op(I::LocalSet(len));
                self.op(I::LocalSet(addr));
                self.op(I::LocalGet(idx));
                self.op(I::LocalGet(len));
                self.op(I::I32GeU);
                self.op(I::If(BlockType::Empty));
                self.trap(&format!("{n}: index out of bounds"));
                self.op(I::End);
                if store {
                    self.store_elem(&t, addr, idx, &val, &mut ta);
                } else {
                    self.load_elem(&t, addr, idx, &mut ta);
                    self.stack.push(t);
                }
                return Ok(Flow::Normal);
            }
            "array.slice" => {
                let v = self.subst.fresh();
                let arr = Ty::Array(Box::new(v.clone()));
                self.pop_expect(n, &[arr.clone(), Ty::I32, Ty::I32], loc)?;
                let t = self.concrete_elem(&v, n, loc)?;
                let mut ta = TempAlloc::default();
                let cnt = self.temp(&mut ta, ValType::I32);
                let start = self.temp(&mut ta, ValType::I32);
                let len = self.temp(&mut ta, ValType::I32);
                let addr = self.temp(&mut ta, ValType::I32);
                for l in [cnt, start, len, addr] {
                    self.op(I::LocalSet(l));
                }
                self.op(I::LocalGet(start));
                self.op(I::LocalGet(len));
                self.op(I::I32GtU);
                self.op(I::LocalGet(cnt));
                self.op(I::LocalGet(len));
                self.op(I::LocalGet(start));
                self.op(I::I32Sub);
                self.op(I::I32GtU);
                self.op(I::I32Or);
                self.op(I::If(BlockType::Empty));
                self.trap("array.slice: range out of bounds");
                self.op(I::End);
                self.op(I::LocalGet(addr));
                self.op(I::LocalGet(start));
                self.op(I::I32Const(t.elem_size() as i32));
                self.op(I::I32Mul);
                self.op(I::I32Add);
                self.op(I::LocalGet(cnt));
                self.stack.push(arr);
                return Ok(Flow::Normal);
            }
            "call" => {
                let Some(top) = self.stack.last().map(|t| self.subst.resolve(t)) else {
                    return Err(self.err(
                        codes::E_STACK_UNDERFLOW,
                        "`call` needs a quotation on top of the stack but the stack is empty",
                        loc,
                    ));
                };
                let Ty::Quot(e) = top else {
                    return Err(self
                        .err(
                            codes::E_TYPE_MISMATCH,
                            format!(
                                "`call` needs a quotation on top of the stack but found `{top}`"
                            ),
                            loc,
                        )
                        .with_stacks(vec!["[ .. -- .. ]".into()], vec![top.to_string()]));
                };
                self.stack.pop();
                let q = Ty::Quot(e.clone()).to_string();
                self.pop_expect(&format!("call {q}"), &e.inputs, loc)?;
                if self.emit {
                    let ti = self.ctx.intern_type(e.wasm_params(), e.wasm_results());
                    self.op(I::CallIndirect {
                        type_index: ti,
                        table_index: 0,
                    });
                }
                self.stack.extend(e.outputs.iter().cloned());
                return Ok(Flow::Normal);
            }
            _ => {}
        }
        // User and library words.
        if let Some(&id) = self.ctx.by_name.get(n) {
            let e = self.ctx.words[id].effect.clone();
            self.pop_expect(n, &e.inputs, loc)?;
            self.op(I::Call(Word::func_index(id)));
            self.callees.push((id, EdgeKind::Call));
            self.stack.extend(e.outputs);
            return Ok(Flow::Normal);
        }
        Err(self.undefined(n, loc))
    }
}
