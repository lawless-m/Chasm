//! The effect checker and code generator.
//!
//! A word body is walked twice by the same [`Walker`]: a checking pass that
//! settles type variables, then an emitting pass that re-walks with the final
//! substitution and produces wasm instructions. Both passes see identical
//! input, so type-variable numbering lines up.

use std::collections::{HashMap, HashSet};

use wasm_encoder::{BlockType, HeapType, Instruction as I, MemArg, ValType};

use crate::ast::{AltArm, Arm, Body, Lit, Node, NodeKind};
use crate::diag::{codes, Diagnostic, Location};
use crate::graph::EdgeKind;
use crate::layout;
use crate::prims;
use crate::types::{
    lower_all, mentions_quot, names, ref_ty, Effect, StructTypes, Subst, Ty, CLOSURE, FRAME,
};

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
    /// A REPL line: wasm type `( ) -> ( )`, its effect is carried on the
    /// memory data stack.
    Line,
    /// A generic word compiled at concrete types (`twice<i32>`); never named
    /// in source, so not in `by_name`.
    Instance,
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
    /// Marked `raw`: the body may reach memory by address.
    pub raw: bool,
    pub origin: Origin,
    pub kind: WordKind,
    pub loc: Location,
    pub callees: Vec<(WordId, EdgeKind)>,
    /// The effect was inferred, not written.
    pub inferred: bool,
    /// A generic word (its effect has type parameters): the template.
    pub generic: Option<Generic>,
    /// For an instance: its template and the type arguments, in the order
    /// of the template's `Effect::params`.
    pub instance_of: Option<(WordId, Vec<Ty>)>,
    /// For a word a `struct` or `union` declaration generated: that type.
    pub generated: Option<String>,
}

/// A generic word's template: its source body, checked once with the type
/// parameters rigid; `None` while it is only declared.
#[derive(Debug, Clone)]
pub struct Generic {
    pub body: Option<Body>,
}

impl Word {
    pub fn func_index(id: WordId) -> u32 {
        FIRST_WORD_FN + id as u32
    }

    /// Reached through a function value: a quotation value or the wrapper
    /// of a `'word`. Its wasm function takes the closure as a last
    /// parameter, after its inputs.
    pub fn takes_env(&self) -> bool {
        self.kind == WordKind::Quote
    }
}

/// Shared compilation state: the word database, type interner, literals.
#[derive(Clone)]
pub struct Ctx {
    pub words: Vec<Word>,
    pub by_name: HashMap<String, WordId>,
    pub types: Vec<TypeDef>,
    type_map: HashMap<(Vec<ValType>, Vec<ValType>), u32>,
    pub literals: Vec<u8>,
    lit_map: HashMap<String, u32>,
    /// Every name defined or declared anywhere in the program, for
    /// "used before it is defined" messages.
    pub all_names: std::collections::HashSet<String>,
    /// Address of the first byte of `literals` in linear memory.
    pub literal_base: u32,
    /// Call words through table 0 (slot = word id) instead of directly,
    /// so a redefinition reaches existing callers (the REPL).
    pub indirect_calls: bool,
    pub struct_types: StructTypes,
    pub structs: Vec<StructDef>,
    pub struct_by_name: HashMap<String, usize>,
    pub unions: Vec<UnionDef>,
    pub union_by_name: HashMap<String, usize>,
    /// Words of the prelude's generic types not made yet, by name: made on
    /// first use (`lookup`) or at the end of the program, so a program that
    /// never uses them keeps its word numbering.
    pub lazy_words: HashMap<String, LazyWord>,
    /// The generated `eq` and `hash` words, by operation and type.
    pub helpers: HashMap<(Op, Ty), WordId>,
    /// Every registered struct or union type, by display name.
    pub registered: HashMap<String, Ty>,
    /// Defer the words of the prelude's generic types (whole programs).
    pub defer_library_generics: bool,
    /// Generic instances made so far: (template, type arguments) to word.
    pub instances: HashMap<(WordId, Vec<Ty>), WordId>,
    /// The type parameters in force while an instance's body is compiled.
    pub type_params: HashMap<String, Ty>,
    /// The body being compiled may use the raw memory words: it is the
    /// prelude's, generated, or a word marked `raw`.
    pub raw: bool,
    /// The wrapper word of each `'word` taken, by the word ticked.
    pub tick_wrappers: HashMap<WordId, WordId>,
    /// Environment types (subtypes of `$closure`), by their fields.
    env_types: HashMap<Vec<ValType>, u32>,
    /// Some emitted code uses `spawn`: modules import the `wack.spawn`
    /// global that hands the host the closure to start.
    pub uses_spawn: bool,
    /// Which words are transformed to unwind and rewind (M12).
    pub unwind: Unwind,
    /// Call-site frame types (subtypes of `$frame`), by their added fields.
    site_frame_types: HashMap<Vec<ValType>, u32>,
}

/// Which words the unwind/rewind transform applies to (M12): none, every
/// word (the native REPL), or the named ones (a whole program's
/// suspendable set).
#[derive(Clone, PartialEq)]
pub enum Unwind {
    Off,
    All,
    Only(std::collections::HashSet<String>),
}

/// A declared struct: its fields in order and its wasm type index.
#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
    pub name: String,
    /// Type parameters, `struct pair T U`.
    pub params: Vec<String>,
    pub fields: Vec<(String, Ty)>,
    pub type_index: u32,
    pub loc: Location,
}

/// `eq` or `hash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    Eq,
    Hash,
}

/// A prelude template waiting to be made: the generic type it belongs to (or
/// its own name), the words made with it (in order), its effect, origin,
/// location, and for a generic `:` word its body.
#[derive(Debug, Clone)]
pub struct LazyWord {
    pub ty: String,
    pub group: Vec<String>,
    pub effect: Effect,
    pub origin: Origin,
    pub loc: Location,
    /// A generic `:` word's body; `None` for the words of a generic type.
    pub body: Option<Body>,
}

/// A declared union: its variants with their fields, in order, and the wasm
/// index of its supertype (variant k is at `type_index + 1 + k`, the GC array
/// of its elements after the last variant).
#[derive(Debug, Clone, PartialEq)]
pub struct UnionDef {
    pub name: String,
    pub params: Vec<String>,
    pub variants: Vec<(String, Vec<(String, Ty)>)>,
    pub type_index: u32,
    pub loc: Location,
}

/// An entry of the module's type section, by index. A struct is followed by
/// its GC array type (`StructArray`, index + 1); the two form one rec group.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeDef {
    Func(Vec<ValType>, Vec<ValType>),
    /// A rec group, at the index of its first member.
    Rec(Vec<Member>),
    /// A further member of the rec group before it.
    Slot,
}

/// A member of a rec group.
#[derive(Debug, Clone, PartialEq)]
pub enum Member {
    /// A struct type, every field mutable.
    Struct {
        fields: Vec<ValType>,
        is_final: bool,
        supertype: Option<u32>,
    },
    /// `array (mut (ref null i))`.
    Array(u32),
}

impl Default for Ctx {
    fn default() -> Self {
        Ctx {
            words: Vec::new(),
            by_name: HashMap::new(),
            types: Vec::new(),
            type_map: HashMap::new(),
            literals: Vec::new(),
            lit_map: HashMap::new(),
            all_names: Default::default(),
            literal_base: layout::LITERALS_BASE,
            indirect_calls: false,
            struct_types: StructTypes::new(),
            structs: Vec::new(),
            struct_by_name: HashMap::new(),
            unions: Vec::new(),
            union_by_name: HashMap::new(),
            lazy_words: HashMap::new(),
            helpers: HashMap::new(),
            registered: HashMap::new(),
            defer_library_generics: false,
            instances: HashMap::new(),
            type_params: HashMap::new(),
            raw: false,
            tick_wrappers: HashMap::new(),
            env_types: HashMap::new(),
            uses_spawn: false,
            unwind: Unwind::Off,
            site_frame_types: HashMap::new(),
        }
    }
}

impl Ctx {
    pub fn intern_type(&mut self, params: Vec<ValType>, results: Vec<ValType>) -> u32 {
        let key = (params, results);
        if let Some(&i) = self.type_map.get(&key) {
            return i;
        }
        let i = self.types.len() as u32;
        self.types.push(TypeDef::Func(key.0.clone(), key.1.clone()));
        self.type_map.insert(key, i);
        i
    }

    /// E_UNKNOWN_TYPE for any struct name in `tys` that is not declared,
    /// except `allow` (the struct being declared, which may name itself).
    pub fn check_types(
        &self,
        tys: &[Ty],
        loc: &Location,
        allow: Option<&str>,
    ) -> Result<(), Diagnostic> {
        for t in tys {
            match t {
                Ty::Struct(n, _) if !self.is_type_name(n) && allow != Some(n.as_str()) => {
                    return Err(Diagnostic::error(
                        codes::E_UNKNOWN_TYPE,
                        format!("unknown type `{n}`; a struct or union can only be used after its declaration, and only types declared above it (or itself) may appear in its fields"),
                        loc.clone(),
                    ));
                }
                Ty::Array(e) => self.check_types(std::slice::from_ref(e), loc, allow)?,
                Ty::Struct(n, args) => {
                    let arity = self.type_arities().get(n).copied();
                    if let Some(k) = arity.filter(|&k| k != args.len()) {
                        return Err(Diagnostic::error(
                            codes::E_UNKNOWN_TYPE,
                            format!(
                                "`{n}` takes {k} type argument{} but `{t}` gives {}",
                                if k == 1 { "" } else { "s" },
                                args.len()
                            ),
                            loc.clone(),
                        ));
                    }
                    self.check_types(args, loc, allow)?
                }
                Ty::Quot(e) => {
                    self.check_types(&e.inputs, loc, allow)?;
                    self.check_types(&e.outputs, loc, allow)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Add a struct type with these (already lowered, all mutable) fields and,
    /// at the next index, the GC array type of that struct. Returns the
    /// struct's index.
    /// Add a rec group; returns the index of its first member.
    pub fn register_rec(&mut self, members: Vec<Member>) -> u32 {
        let i = self.types.len() as u32;
        let n = members.len();
        self.types.push(TypeDef::Rec(members));
        for _ in 1..n {
            self.types.push(TypeDef::Slot);
        }
        i
    }

    /// The number of type parameters of each declared struct or union, for
    /// the parser to read applied types such as `pair i32 str`.
    pub fn type_arities(&self) -> HashMap<String, usize> {
        self.structs
            .iter()
            .map(|s| (s.name.clone(), s.params.len()))
            .chain(self.unions.iter().map(|u| (u.name.clone(), u.params.len())))
            .collect()
    }

    /// Whether `name` is a declared struct or union.
    pub fn is_type_name(&self, name: &str) -> bool {
        self.struct_by_name.contains_key(name) || self.union_by_name.contains_key(name)
    }

    /// Whether any struct or union type is registered (step modules then
    /// import `wack.refs`).
    pub fn has_ref_types(&self) -> bool {
        self.types.iter().any(|t| matches!(t, TypeDef::Rec(_)))
    }

    /// The field types of a declared struct (one list) or union (one list
    /// per variant) at type arguments `args`.
    fn type_fields(&self, name: &str, args: &[Ty]) -> Option<(bool, Vec<Vec<Ty>>)> {
        let (params, lists, union): (&[String], Vec<Vec<Ty>>, bool) =
            if let Some(&k) = self.struct_by_name.get(name) {
                let s = &self.structs[k];
                (
                    &s.params,
                    vec![s.fields.iter().map(|(_, t)| t.clone()).collect()],
                    false,
                )
            } else {
                let u = &self.unions[*self.union_by_name.get(name)?];
                let lists = u
                    .variants
                    .iter()
                    .map(|(_, f)| f.iter().map(|(_, t)| t.clone()).collect())
                    .collect();
                (&u.params, lists, true)
            };
        let map: HashMap<String, Ty> = params.iter().cloned().zip(args.iter().cloned()).collect();
        let lists = lists
            .into_iter()
            .map(|l| l.iter().map(|t| t.substitute(&map)).collect())
            .collect();
        Some((union, lists))
    }

    /// The unregistered concrete struct and union types the fields of `t`
    /// mention directly.
    fn type_deps(&self, t: &Ty) -> Vec<Ty> {
        let Ty::Struct(name, args) = t else {
            return Vec::new();
        };
        let Some((_, lists)) = self.type_fields(name, args) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for f in lists.iter().flatten() {
            crate::types::applied_types(f, &mut out);
        }
        out.retain(|d| {
            !self.struct_types.contains_key(&d.to_string()) && !d.has_var() && !d.has_param()
        });
        out
    }

    /// Register a concrete struct or union type (a declared one, or a
    /// generic one at type arguments) if it is not yet, and return its
    /// index. Types its fields need are registered first, each in its own
    /// rec group, except those that refer back to `t`: they share `t`'s
    /// group. `None` for a type that is not concrete or not declared.
    pub fn register_type(&mut self, t: &Ty) -> Option<u32> {
        let Ty::Struct(name, _) = t else {
            return None;
        };
        if let Some(&(i, _)) = self.struct_types.get(&t.to_string()) {
            return Some(i);
        }
        if t.has_var() || t.has_param() || !self.is_type_name(name) {
            return None;
        }
        // Every unregistered type reachable from `t`, and its direct deps.
        let mut reach: Vec<Ty> = vec![t.clone()];
        let mut deps: Vec<Vec<Ty>> = Vec::new();
        let mut k = 0;
        while k < reach.len() {
            let d = self.type_deps(&reach[k]);
            for x in &d {
                if !reach.contains(x) {
                    reach.push(x.clone());
                }
            }
            deps.push(d);
            k += 1;
        }
        let reaches_t = |from: usize| -> bool {
            let mut seen = vec![from];
            let mut i = 0;
            while i < seen.len() {
                for d in &deps[seen[i]] {
                    let j = reach.iter().position(|r| r == d).unwrap();
                    if j == 0 {
                        return true;
                    }
                    if !seen.contains(&j) {
                        seen.push(j);
                    }
                }
                i += 1;
            }
            false
        };
        let in_group: Vec<bool> = (0..reach.len()).map(|i| i == 0 || reaches_t(i)).collect();
        for (i, r) in reach.iter().enumerate() {
            if !in_group[i] {
                self.register_type(r);
            }
        }
        let group: Vec<Ty> = reach
            .iter()
            .zip(&in_group)
            .filter(|(_, &g)| g)
            .map(|(r, _)| r.clone())
            .collect();
        // A field holding a function value needs `$closure` first.
        let quot = group.iter().any(|g| {
            let Ty::Struct(n, args) = g else {
                unreachable!()
            };
            let (_, lists) = self.type_fields(n, args).unwrap();
            lists.iter().flatten().any(mentions_quot)
        });
        if quot {
            self.closure_type();
        }
        // Indices first, so fields inside the group resolve.
        let mut next = self.types.len() as u32;
        let mut layouts = Vec::new();
        for g in &group {
            let Ty::Struct(n, args) = g else {
                unreachable!()
            };
            let (union, lists) = self.type_fields(n, args).unwrap();
            let size = if union { lists.len() as u32 + 2 } else { 2 };
            self.struct_types
                .insert(g.to_string(), (next, next + size - 1));
            self.registered.insert(g.to_string(), g.clone());
            layouts.push((next, union, lists));
            next += size;
        }
        let mut members = Vec::new();
        for (base, union, lists) in layouts {
            let lower = |l: &[Ty]| -> Vec<ValType> { lower_all(l, &self.struct_types) };
            if union {
                members.push(Member::Struct {
                    fields: Vec::new(),
                    is_final: false,
                    supertype: None,
                });
                for l in &lists {
                    members.push(Member::Struct {
                        fields: lower(l),
                        is_final: true,
                        supertype: Some(base),
                    });
                }
            } else {
                members.push(Member::Struct {
                    fields: lower(&lists[0]),
                    is_final: true,
                    supertype: None,
                });
            }
            members.push(Member::Array(base));
        }
        self.register_rec(members);
        // A value of a generic type's instance can always be rendered: its
        // readers (and a union's `tag`) exist at these arguments.
        for g in &group {
            let Ty::Struct(n, args) = g else {
                unreachable!()
            };
            let (params, names) = if let Some(&k) = self.union_by_name.get(n) {
                let u = &self.unions[k];
                let mut names = vec![format!("{n}.tag")];
                for (v, fields) in &u.variants {
                    names.extend(fields.iter().map(|(f, _)| format!("{n}.{v}.{f}")));
                }
                (u.params.clone(), names)
            } else {
                let s = &self.structs[self.struct_by_name[n]];
                let names: Vec<String> = s.fields.iter().map(|(f, _)| format!("{n}.{f}")).collect();
                (s.params.clone(), names)
            };
            if params.is_empty() {
                continue;
            }
            let map: HashMap<String, Ty> =
                params.iter().cloned().zip(args.iter().cloned()).collect();
            for w in names {
                let Some(id) = self.lookup(&w) else {
                    continue;
                };
                let a: Vec<Ty> = self.words[id]
                    .effect
                    .params()
                    .iter()
                    .map(|p| map[p].clone())
                    .collect();
                let _ = instantiate(self, id, &a);
            }
        }
        self.struct_types.get(&t.to_string()).map(|&(i, _)| i)
    }

    /// The index of `$closure`, registered on first use only, so a program
    /// without function values compiles as before.
    pub fn closure_type(&mut self) -> u32 {
        if let Some(&(i, _)) = self.struct_types.get(CLOSURE) {
            return i;
        }
        let i = self.types.len() as u32;
        self.register_rec(vec![
            Member::Struct {
                fields: vec![ValType::I32],
                is_final: false,
                supertype: None,
            },
            Member::Array(i),
        ]);
        self.struct_types.insert(CLOSURE.to_string(), (i, i + 1));
        i
    }

    /// The environment type with these fields (the slot first), a final
    /// subtype of `$closure` (index `closure`), in a rec group of its own.
    pub fn env_type(&mut self, fields: Vec<ValType>, closure: u32) -> u32 {
        if let Some(&t) = self.env_types.get(&fields) {
            return t;
        }
        let t = self.register_rec(vec![Member::Struct {
            fields: fields.clone(),
            is_final: true,
            supertype: Some(closure),
        }]);
        self.env_types.insert(fields, t);
        t
    }

    /// Whether the word named `word_name` is transformed to unwind and rewind.
    pub fn transforms(&self, word_name: &str) -> bool {
        match &self.unwind {
            Unwind::Off => false,
            Unwind::All => true,
            Unwind::Only(s) => s.contains(word_name),
        }
    }

    /// The index of `$frame` (next, site), registered on first use only.
    pub fn frame_type(&mut self) -> u32 {
        if let Some(&(i, _)) = self.struct_types.get(FRAME) {
            return i;
        }
        let i = self.types.len() as u32;
        self.register_rec(vec![Member::Struct {
            fields: vec![ref_ty(i), ValType::I32],
            is_final: false,
            supertype: None,
        }]);
        self.struct_types.insert(FRAME.to_string(), (i, i));
        i
    }

    /// The frame type of a call site: a final subtype of `$frame` adding
    /// `fields` (the function's locals, params first, then the operand
    /// values below the call), in a rec group of its own.
    pub fn site_frame_type(&mut self, fields: Vec<ValType>) -> u32 {
        if let Some(&t) = self.site_frame_types.get(&fields) {
            return t;
        }
        let frame = self.frame_type();
        let mut all = vec![ref_ty(frame), ValType::I32];
        all.extend(fields.iter().copied());
        let t = self.register_rec(vec![Member::Struct {
            fields: all,
            is_final: true,
            supertype: Some(frame),
        }]);
        self.site_frame_types.insert(fields, t);
        t
    }

    /// The wasm function type of a word with effect `e`; `env` adds the
    /// closure parameter of a word reached through a function value.
    pub fn func_type(&mut self, e: &Effect, env: bool) -> u32 {
        self.register_effect(e);
        let mut params = e.wasm_params(&self.struct_types);
        if env {
            params.push(ref_ty(self.closure_type()));
        }
        let results = e.wasm_results(&self.struct_types);
        self.intern_type(params, results)
    }

    /// Register every concrete struct or union type `tys` mention, and
    /// `$closure` if they mention a function value.
    pub fn register_types(&mut self, tys: &[Ty]) {
        if tys.iter().any(mentions_quot) {
            self.closure_type();
        }
        let mut found = Vec::new();
        for t in tys {
            crate::types::applied_types(t, &mut found);
        }
        for t in found {
            self.register_type(&t);
        }
    }

    pub fn register_effect(&mut self, e: &Effect) {
        self.register_types(&e.inputs);
        self.register_types(&e.outputs);
    }

    pub fn register_struct_type(&mut self, fields: Vec<ValType>) -> u32 {
        let i = self.types.len() as u32;
        self.register_rec(vec![
            Member::Struct {
                fields,
                is_final: true,
                supertype: None,
            },
            Member::Array(i),
        ])
    }

    /// Place a string literal in read-only data; returns (addr, len).
    pub fn intern_str(&mut self, s: &str) -> (i32, i32) {
        let len = s.len() as i32;
        if let Some(&off) = self.lit_map.get(s) {
            return ((self.literal_base + off) as i32, len);
        }
        let off = self.literals.len() as u32;
        self.literals.extend_from_slice(s.as_bytes());
        self.lit_map.insert(s.to_string(), off);
        ((self.literal_base + off) as i32, len)
    }

    /// Start a fresh literal window at `base` (a REPL step's heap pointer).
    /// Dedup is per window; earlier addresses stay valid because the host
    /// has already written their bytes.
    pub fn begin_literals(&mut self, base: u32) {
        self.literals.clear();
        self.lit_map.clear();
        self.literal_base = base;
    }

    /// The generated word for `eq` or `hash` on the concrete type `t`
    /// (`eq<point>`), made on first use. It is added before its body is
    /// compiled, so a recursive type's helper finds itself.
    pub fn hash_eq_word(&mut self, op: Op, t: &Ty, loc: &Location) -> Result<WordId, Diagnostic> {
        if let Some(&id) = self.helpers.get(&(op, t.clone())) {
            return Ok(id);
        }
        let (name, effect) = match op {
            Op::Eq => (
                format!("eq<{t}>"),
                Effect::new(vec![t.clone(), t.clone()], vec![Ty::I32]),
            ),
            Op::Hash => (
                format!("hash<{t}>"),
                Effect::new(vec![t.clone()], vec![Ty::I32]),
            ),
        };
        let id = self.add_word(Word {
            name: name.clone(),
            effect: effect.clone(),
            body: None,
            failed: false,
            export: false,
            raw: false,
            origin: Origin::Library,
            kind: WordKind::Instance,
            loc: loc.clone(),
            callees: Vec::new(),
            inferred: false,
            generic: None,
            instance_of: None,
            generated: None,
        });
        self.helpers.insert((op, t.clone()), id);
        self.register_types(std::slice::from_ref(t));
        let src = crate::hasheq::helper_source(self, op, t);
        let internal = |d: Diagnostic| {
            Diagnostic::error(
                codes::E_INTERNAL,
                format!("the generated `{name}` does not compile: {}", d.message),
                loc.clone(),
            )
        };
        let toks = crate::lexer::lex(&name, &src).map_err(internal)?;
        let body = match crate::parser::parse_repl_with(&name, &toks, &self.type_arities())
            .map_err(internal)?
        {
            crate::parser::ReplInput::Body(b) => b,
            crate::parser::ReplInput::Items(..) => unreachable!("a generated body has no items"),
        };
        let raw = std::mem::replace(&mut self.raw, true);
        let out = compile_body(self, &name, Mode::Declared(&effect), &body, loc, &[]);
        self.raw = raw;
        let out = out.map_err(internal)?;
        let w = &mut self.words[id];
        w.body = Some(out.compiled);
        w.callees = out.callees;
        Ok(id)
    }

    /// The word named `n`, making it first if it is a pending prelude
    /// template (with the rest of its type's words).
    pub fn lookup(&mut self, n: &str) -> Option<WordId> {
        if let Some(&id) = self.by_name.get(n) {
            return Some(id);
        }
        let group = self.lazy_words.get(n)?.group.clone();
        for w in group {
            let l = self.lazy_words.remove(&w).unwrap();
            let mut word = Word {
                name: w.clone(),
                effect: l.effect,
                body: None,
                failed: false,
                export: false,
                raw: false,
                origin: l.origin,
                kind: WordKind::Named,
                loc: l.loc,
                callees: Vec::new(),
                inferred: false,
                generic: Some(Generic {
                    body: l.body.clone(),
                }),
                instance_of: None,
                generated: if l.body.is_some() { None } else { Some(l.ty) },
            };
            // Never emitted by `assemble`; no literal is interned for it.
            word.body = Some(Compiled {
                locals: Vec::new(),
                code: vec![I::Unreachable],
            });
            self.add_word(word);
        }
        self.by_name.get(n).copied()
    }

    /// Make every pending prelude template.
    pub fn make_lazy_words(&mut self) {
        let mut names: Vec<(String, String)> = self
            .lazy_words
            .iter()
            .map(|(n, l)| (l.ty.clone(), n.clone()))
            .collect();
        names.sort();
        for (_, n) in names {
            self.lookup(&n);
        }
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
    /// As `Declared`, for a word reached through a function value: the
    /// closure follows the inputs as a last parameter.
    Closure(&'a Effect),
    Derived,
    /// A REPL line: inputs are the types on the memory data stack; it loads
    /// them, runs, and stores its outputs back.
    Line(&'a [Ty]),
    /// Inference: `n` inputs of fresh type variables, outputs whatever the
    /// body leaves. Unresolved variables are allowed in the result; the
    /// caller generalises them. Check-only (see `check_body`).
    Infer(usize),
}

pub struct Output {
    pub effect: Effect,
    pub compiled: Compiled,
    pub callees: Vec<(WordId, EdgeKind)>,
    /// The locals of enclosing words the body captures, in field order.
    pub captures: Vec<String>,
    /// The environment type holding them, if any.
    pub env_type: Option<u32>,
}

/// A local of an enclosing word, visible in a quotation value.
#[derive(Debug, Clone)]
pub struct Capture {
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
}

/// Compile one body. `outer_locals` are the locals of enclosing words,
/// which a quotation value captures.
pub fn compile_body(
    ctx: &mut Ctx,
    name: &str,
    mode: Mode<'_>,
    body: &Body,
    loc: &Location,
    outer_locals: &[Capture],
) -> Result<Output, Diagnostic> {
    let mut first = Walker::new(ctx, name, false, Subst::default(), outer_locals);
    let effect = first.run(&mode, body, loc)?;
    let mut subst = first.subst;
    subst.restart();
    let captures = first.captures;
    let mut second = Walker::new(ctx, name, true, subst, outer_locals);
    // The emitting pass knows the environment before it reads from it.
    second.captures = captures.clone();
    let effect2 = second.run(&mode, body, loc)?;
    debug_assert_eq!(captures, second.captures);
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
        captures,
        env_type: second.env_view.map(|(t, _)| t),
    })
}

/// Check one body without emitting code or adding words: the first pass of
/// `compile_body` only. Returns the body's effect.
pub fn check_body(
    ctx: &mut Ctx,
    name: &str,
    mode: Mode<'_>,
    body: &Body,
    loc: &Location,
    outer_locals: &[Capture],
) -> Result<Effect, Diagnostic> {
    let words = ctx.words.len();
    let mut walker = Walker::new(ctx, name, false, Subst::default(), outer_locals);
    let effect = walker.run(&mode, body, loc);
    debug_assert!(
        ctx.words[words..].iter().all(|w| w.generic.is_some()),
        "a check-only pass added words other than pending templates"
    );
    effect
}

/// The instance of generic word `generic` at type arguments `args` (in
/// `Effect::params` order): an existing one, or a new word compiled from the
/// template's body at those types. The instance is registered before its
/// body is compiled, so a generic word using itself at the same types finds
/// it.
pub fn instantiate(ctx: &mut Ctx, generic: WordId, args: &[Ty]) -> Result<WordId, Diagnostic> {
    let key = (generic, args.to_vec());
    if let Some(&id) = ctx.instances.get(&key) {
        return Ok(id);
    }
    let t = &ctx.words[generic];
    let map: HashMap<String, Ty> = t
        .effect
        .params()
        .into_iter()
        .zip(args.iter().cloned())
        .collect();
    let arg_names: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let word = Word {
        name: format!("{}<{}>", t.name, arg_names.join(",")),
        effect: t.effect.substitute(&map),
        body: None,
        failed: false,
        export: false,
        raw: false,
        origin: t.origin,
        kind: WordKind::Instance,
        loc: t.loc.clone(),
        callees: vec![(generic, EdgeKind::Call)],
        inferred: false,
        generic: None,
        instance_of: Some(key.clone()),
        generated: t.generated.clone(),
    };
    let id = ctx.add_word(word);
    ctx.instances.insert(key, id);
    let effect = ctx.words[id].effect.clone();
    ctx.register_effect(&effect);
    if ctx.words[generic].generated.is_some() {
        generate_instance(ctx, id, &map);
        return Ok(id);
    }
    compile_instance(ctx, id)?;
    Ok(id)
}

/// The code of an instance of a word a generic struct or union generated:
/// the type at the instance's type arguments is registered and the word's
/// code generated for it.
fn generate_instance(ctx: &mut Ctx, id: WordId, map: &HashMap<String, Ty>) {
    let (generic, _) = ctx.words[id].instance_of.clone().unwrap();
    let ty = ctx.words[generic].generated.clone().unwrap();
    let params = match ctx.struct_by_name.get(&ty) {
        Some(&k) => ctx.structs[k].params.clone(),
        None => ctx.unions[ctx.union_by_name[&ty]].params.clone(),
    };
    let args: Vec<Ty> = params
        .iter()
        .map(|p| map.get(p).cloned().unwrap_or(Ty::I32))
        .collect();
    let idx = ctx
        .register_type(&Ty::Struct(ty.clone(), args.clone()))
        .expect("a concrete instantiation registers");
    let pmap: HashMap<String, Ty> = params.iter().cloned().zip(args).collect();
    let subst = |fields: &[(String, Ty)]| -> Vec<(String, Ty)> {
        fields
            .iter()
            .map(|(n, t)| (n.clone(), t.substitute(&pmap)))
            .collect()
    };
    let words = match ctx.struct_by_name.get(&ty) {
        Some(&k) => {
            let fields = subst(&ctx.structs[k].fields);
            crate::program::struct_words(&ty, &[], &fields, idx)
        }
        None => {
            let mut def = ctx.unions[ctx.union_by_name[&ty]].clone();
            def.variants = def
                .variants
                .iter()
                .map(|(v, f)| (v.clone(), subst(f)))
                .collect();
            def.params = Vec::new();
            crate::program::union_words(ctx, &def, idx, true)
        }
    };
    let template = ctx.words[generic].name.clone();
    let code = words
        .into_iter()
        .find(|(w, _, _)| *w == template)
        .map(|(_, _, c)| c)
        .unwrap();
    let w = &mut ctx.words[id];
    w.body = Some(Compiled {
        locals: vec![],
        code,
    });
    w.failed = false;
}

/// (Re)compile an instance from its template's body at its type arguments,
/// in place. A template with no body yet leaves the instance unresolved.
pub fn compile_instance(ctx: &mut Ctx, id: WordId) -> Result<(), Diagnostic> {
    let Some((generic, args)) = ctx.words[id].instance_of.clone() else {
        return Ok(());
    };
    let Some(body) = ctx.words[generic]
        .generic
        .as_ref()
        .and_then(|g| g.body.clone())
    else {
        return Ok(());
    };
    let map: HashMap<String, Ty> = ctx.words[generic]
        .effect
        .params()
        .into_iter()
        .zip(args)
        .collect();
    let name = ctx.words[id].name.clone();
    let effect = ctx.words[id].effect.clone();
    let loc = ctx.words[generic].loc.clone();
    let saved = std::mem::replace(&mut ctx.type_params, map);
    let template = &ctx.words[generic];
    let raw = template.raw || template.origin == Origin::Library;
    let saved_raw = std::mem::replace(&mut ctx.raw, raw);
    let out = compile_body(ctx, &name, Mode::Declared(&effect), &body, &loc, &[]);
    ctx.raw = saved_raw;
    ctx.type_params = saved;
    let w = &mut ctx.words[id];
    match out {
        Ok(out) => {
            w.body = Some(out.compiled);
            w.callees = out.callees;
            w.callees.push((generic, EdgeKind::Call));
            w.failed = false;
            Ok(())
        }
        Err(d) => {
            w.failed = true;
            Err(d)
        }
    }
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

/// A combinator's array: a linear `addr` local, or a struct-array view (GC
/// array in `arr`, `start` local unless the view starts at 0, type `ti`).
#[derive(Clone, Copy)]
enum ArrLocals {
    Linear(u32),
    Gc {
        arr: u32,
        start: Option<u32>,
        ti: u32,
    },
}

/// Temps are reused per exact value type, so a `(ref null $point)` temp is
/// never shared with an `f64` or another struct's reference.
#[derive(Default)]
struct TempAlloc {
    counts: HashMap<ValType, u32>,
}

fn vt_size(vt: ValType) -> u32 {
    match vt {
        ValType::Ref(_) => panic!("references are never stored in linear memory"),
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
    outer_locals: Vec<Capture>,
    nparams: u32,
    local_types: Vec<ValType>,
    temps: HashMap<(ValType, u32), u32>,
    code: Vec<I<'static>>,
    depth: u32,
    loops: Vec<LoopCtx>,
    callees: Vec<(WordId, EdgeKind)>,
    /// The local holding the closure, in a word reached through a function
    /// value.
    env: Option<u32>,
    /// The outer locals read so far (checking pass) or all of them
    /// (emitting pass), in environment field order.
    captures: Vec<String>,
    /// In the emitting pass of a body with captures: the environment type
    /// and the local holding the closure cast to it.
    env_view: Option<(u32, u32)>,
    /// The emitting pass of a function transformed to unwind and rewind.
    transformed: bool,
    /// In a transformed function: the wasm types of its parameters, the
    /// locals holding the popped frame and the site being resumed, the
    /// stack at entry, and its suspendable call sites in emission order.
    param_types: Vec<ValType>,
    frame_local: u32,
    resume_local: u32,
    entry_stack: Vec<ValType>,
    sites: Vec<Site>,
}

/// An arm of `match` or `alt` walked into its own buffer: how it ends, the
/// stack it leaves, its code, and its sites (first index, count).
type WalkedArm = (Flow, Vec<Ty>, Vec<I<'static>>, (u32, u32));

/// A suspendable call site of a transformed function: the operand values
/// below the call's inputs, its outputs, and the scratch locals the
/// operands are popped into when unwinding.
struct Site {
    below: Vec<ValType>,
    outputs: Vec<ValType>,
    scratch: Vec<u32>,
}

/// Placeholders in a transformed function's code, replaced once its locals
/// are final: the entry restore, and each site's operand rebuild
/// (`MARK + 1 + 2k`) and unwind save (`MARK + 2 + 2k`).
const MARK: u32 = 0xF000_0000;

/// `[mode] == value`, the unwind mode cell.
fn mode_is(value: i32) -> [I<'static>; 4] {
    [
        I::I32Const(0),
        I::I32Load(memarg(layout::UNWIND_MODE, ValType::I32)),
        I::I32Const(value),
        I::I32Eq,
    ]
}

/// A zero or null of `vt`, standing in for a value not computed.
fn dummy(vt: ValType) -> I<'static> {
    match vt {
        ValType::I32 => I::I32Const(0),
        ValType::I64 => I::I64Const(0),
        ValType::F32 => I::F32Const(0.0.into()),
        ValType::F64 => I::F64Const(0.0.into()),
        ValType::Ref(r) => I::RefNull(r.heap_type),
        ValType::V128 => unreachable!("no v128 values"),
    }
}

impl<'c> Walker<'c> {
    fn new(
        ctx: &'c mut Ctx,
        name: &str,
        emit: bool,
        subst: Subst,
        outer_locals: &[Capture],
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
            env: None,
            captures: Vec::new(),
            env_view: None,
            transformed: false,
            param_types: Vec::new(),
            frame_local: 0,
            resume_local: 0,
            entry_stack: Vec::new(),
            sites: Vec::new(),
        }
    }

    fn run(&mut self, mode: &Mode<'_>, body: &Body, loc: &Location) -> Result<Effect, Diagnostic> {
        self.transformed = self.emit && self.ctx.transforms(&self.name);
        let effect = self.run_body(mode, body, loc)?;
        if self.transformed {
            let results = if matches!(mode, Mode::Line(_)) {
                Vec::new()
            } else {
                self.lower_all(&effect.outputs)
            };
            self.splice_unwind(&results);
        }
        Ok(effect)
    }

    /// A block type from lowered params and results.
    fn block_type_vt(&mut self, p: Vec<ValType>, r: Vec<ValType>) -> BlockType {
        if p.is_empty() && r.is_empty() {
            return BlockType::Empty;
        }
        if p.is_empty() && r.len() == 1 {
            return BlockType::Result(r[0]);
        }
        BlockType::FunctionType(self.ctx.intern_type(p, r))
    }

    /// Emit a call with wasm inputs `ins` and outputs `outs` (the checker
    /// stack already below its inputs). In a transformed function it is a
    /// suspendable site (design steps 5 and 1): rewinding to it, the stack
    /// is rebuilt from the frame and `rewound` (after dummies of
    /// `rewound_ins`) calls again; rewinding to a later site, it is skipped;
    /// after it, an unwinding callee makes this function save its frame and
    /// return.
    fn site_call(
        &mut self,
        ins: Vec<ValType>,
        outs: Vec<ValType>,
        call: Vec<I<'static>>,
        rewound: Vec<I<'static>>,
        rewound_ins: Vec<ValType>,
    ) {
        if !self.transformed {
            self.code.extend(call);
            return;
        }
        let stack = self.stack.clone();
        let below = self.lower_all(&stack);
        let k = self.sites.len() as u32;
        let scratch = below.iter().map(|&vt| self.new_local(vt)).collect();
        let mut pin = below.clone();
        pin.extend(&ins);
        let mut pout = below.clone();
        pout.extend(&outs);
        let bt = self.block_type_vt(pin.clone(), pout);
        self.code.extend(mode_is(layout::REWINDING));
        self.op(I::If(bt));
        self.op(I::LocalGet(self.resume_local));
        self.op(I::I32Const(k as i32));
        self.op(I::I32Eq);
        self.op(I::If(bt));
        for _ in &pin {
            self.op(I::Drop);
        }
        self.op(I::Call(MARK + 1 + 2 * k));
        for &vt in &rewound_ins {
            self.op(dummy(vt));
        }
        self.code.extend(rewound);
        self.op(I::Else);
        for _ in &ins {
            self.op(I::Drop);
        }
        for &vt in &outs {
            self.op(dummy(vt));
        }
        self.op(I::End);
        self.op(I::Else);
        self.code.extend(call);
        self.op(I::End);
        self.op(I::Call(MARK + 2 + 2 * k));
        self.sites.push(Site {
            below,
            outputs: outs,
            scratch,
        });
    }

    /// A call to word `id` (effect `e`): direct, or through table 0 in the
    /// REPL. A suspendable site when the callee is transformed, and every
    /// table call is.
    fn word_call(&mut self, id: WordId, e: &Effect) {
        self.callees.push((id, EdgeKind::Call));
        let direct = !self.ctx.indirect_calls;
        if direct && !(self.transformed && self.ctx.transforms(&self.ctx.words[id].name)) {
            self.op(I::Call(Word::func_index(id)));
            return;
        }
        self.ctx.register_effect(e);
        let ins = e.wasm_params(&self.ctx.struct_types);
        let outs = e.wasm_results(&self.ctx.struct_types);
        let call = if direct {
            vec![I::Call(Word::func_index(id))]
        } else {
            let ti = self.ctx.intern_type(ins.clone(), outs.clone());
            vec![
                I::I32Const(id as i32),
                I::CallIndirect {
                    type_index: ti,
                    table_index: 0,
                },
            ]
        };
        self.site_call(ins.clone(), outs, call.clone(), call, ins);
    }

    /// Replace the placeholders of a transformed function now that its
    /// locals are final: each site's frame type holds every local (params
    /// first) and then the operands below the call.
    fn splice_unwind(&mut self, results: &[ValType]) {
        let frame = self.ctx.frame_type();
        let locals: Vec<ValType> = self
            .param_types
            .iter()
            .chain(&self.local_types)
            .copied()
            .collect();
        let n = locals.len() as u32;
        let types: Vec<u32> = self
            .sites
            .iter()
            .map(|s| locals.iter().chain(&s.below).copied().collect())
            .collect::<Vec<Vec<ValType>>>()
            .into_iter()
            .map(|f| self.ctx.site_frame_type(f))
            .collect();
        let (fl, rl) = (self.frame_local, self.resume_local);
        let cast = |t: u32| I::RefCastNullable(HeapType::Concrete(t));
        let code = std::mem::take(&mut self.code);
        let mut out = Vec::with_capacity(code.len());
        for i in code {
            let I::Call(x) = i else {
                out.push(i);
                continue;
            };
            if x < MARK {
                out.push(i);
            } else if x == MARK {
                // Step 2: pop this function's frame and restore its locals.
                if self.sites.is_empty() {
                    continue;
                }
                let s = self.entry_stack.clone();
                let bt = self.block_type_vt(s.clone(), s);
                out.extend(mode_is(layout::REWINDING));
                out.push(I::If(bt));
                out.extend([
                    I::GlobalGet(0),
                    cast(frame),
                    I::LocalTee(fl),
                    I::StructGet {
                        struct_type_index: frame,
                        field_index: 0,
                    },
                    I::GlobalSet(0),
                    I::LocalGet(fl),
                    I::StructGet {
                        struct_type_index: frame,
                        field_index: 1,
                    },
                    I::LocalSet(rl),
                ]);
                for (k, &t) in types.iter().enumerate() {
                    out.extend([
                        I::LocalGet(rl),
                        I::I32Const(k as i32),
                        I::I32Eq,
                        I::If(BlockType::Empty),
                    ]);
                    for j in (0..n).filter(|&j| j != fl && j != rl) {
                        out.extend([
                            I::LocalGet(fl),
                            cast(t),
                            I::StructGet {
                                struct_type_index: t,
                                field_index: 2 + j,
                            },
                            I::LocalSet(j),
                        ]);
                    }
                    out.push(I::End);
                }
                out.push(I::End);
            } else if (x - MARK) % 2 == 1 {
                // Step 5: the saved operands below the call.
                let k = ((x - MARK - 1) / 2) as usize;
                let t = types[k];
                for j in 0..self.sites[k].below.len() as u32 {
                    out.extend([
                        I::LocalGet(fl),
                        cast(t),
                        I::StructGet {
                            struct_type_index: t,
                            field_index: 2 + n + j,
                        },
                    ]);
                }
            } else {
                // Step 1: unwinding, save this function's frame and return.
                let k = ((x - MARK - 2) / 2) as usize;
                let t = types[k];
                let site = &self.sites[k];
                let mut p = site.below.clone();
                p.extend(&site.outputs);
                let (nouts, scratch) = (site.outputs.len(), site.scratch.clone());
                let bt = self.block_type_vt(p.clone(), p);
                out.extend(mode_is(layout::UNWINDING));
                out.push(I::If(bt));
                out.extend((0..nouts).map(|_| I::Drop));
                out.extend(scratch.iter().rev().map(|&l| I::LocalSet(l)));
                out.extend([I::GlobalGet(0), cast(frame), I::I32Const(k as i32)]);
                out.extend((0..n).map(I::LocalGet));
                out.extend(scratch.iter().map(|&l| I::LocalGet(l)));
                out.extend([I::StructNew(t), I::GlobalSet(0)]);
                out.extend(results.iter().map(|&vt| dummy(vt)));
                out.extend([I::Return, I::End]);
            }
        }
        self.code = out;
    }

    fn run_body(
        &mut self,
        mode: &Mode<'_>,
        body: &Body,
        loc: &Location,
    ) -> Result<Effect, Diagnostic> {
        if let Mode::Declared(e) | Mode::Closure(e) = mode {
            self.stack = e.inputs.clone();
            if self.emit {
                self.ctx.register_effect(e);
            }
            let params = e.wasm_params(&self.ctx.struct_types);
            self.nparams = params.len() as u32;
            for i in 0..self.nparams {
                self.op(I::LocalGet(i));
            }
            if matches!(mode, Mode::Closure(_)) {
                let env = self.nparams;
                self.env = Some(env);
                self.nparams += 1;
                if self.emit && !self.captures.is_empty() {
                    let c = self.ctx.closure_type();
                    let mut fields = vec![ValType::I32];
                    for n in self.captures.clone() {
                        let ty = self.capture(&n).expect("a recorded capture").ty;
                        fields.extend(self.lower(&ty));
                    }
                    let t = self.ctx.env_type(fields, c);
                    let local = self.new_local(ref_ty(t));
                    self.op(I::LocalGet(env));
                    self.op(I::RefCastNullable(HeapType::Concrete(t)));
                    self.op(I::LocalSet(local));
                    self.env_view = Some((t, local));
                }
            }
        }
        if self.transformed {
            if let Mode::Declared(e) | Mode::Closure(e) = mode {
                self.param_types = e.wasm_params(&self.ctx.struct_types);
                self.entry_stack = self.lower_all(&e.inputs);
                if matches!(mode, Mode::Closure(_)) {
                    let c = self.ctx.closure_type();
                    self.param_types.push(ref_ty(c));
                }
            }
            let frame = self.ctx.frame_type();
            self.frame_local = self.new_local(ref_ty(frame));
            self.resume_local = self.new_local(ValType::I32);
            self.op(I::Call(MARK));
        }
        let mut line_base = None;
        if let Mode::Line(inputs) = mode {
            if self.transformed {
                self.skip_in_rewind(|w| {
                    w.stack = inputs.to_vec();
                    line_base = Some(w.line_prologue(inputs));
                    Ok(Flow::Normal)
                })?;
            } else {
                self.stack = inputs.to_vec();
                line_base = Some(self.line_prologue(inputs));
            }
        }
        let mut infer_inputs = Vec::new();
        if let Mode::Infer(n) = mode {
            infer_inputs = (0..*n).map(|_| self.subst.fresh()).collect();
            self.stack = infer_inputs.clone();
        }
        let flow = self.seq(body)?;
        match mode {
            Mode::Declared(e) | Mode::Closure(e) => {
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
            Mode::Line(inputs) => {
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
                if flow == Flow::Normal {
                    self.line_epilogue(line_base.unwrap(), &outputs);
                }
                Ok(Effect::new(inputs.to_vec(), outputs))
            }
            Mode::Infer(_) => {
                let outputs = if flow == Flow::Normal {
                    self.resolved_stack()
                } else {
                    Vec::new()
                };
                Ok(Effect::new(self.subst.resolve_all(&infer_inputs), outputs))
            }
        }
    }

    /// Pop a line's inputs off the memory data stack onto the wasm stack.
    /// Returns the local holding the base address of those slots.
    fn line_prologue(&mut self, inputs: &[Ty]) -> u32 {
        let vts = self.lower_all(inputs);
        let base = self.new_local(ValType::I32);
        self.op(I::I32Const(0));
        self.op(I::I32Load(memarg(layout::DATA_STACK_PTR, ValType::I32)));
        self.op(I::I32Const((layout::STACK_SLOT * vts.len() as u32) as i32));
        self.op(I::I32Sub);
        self.op(I::LocalSet(base));
        self.op(I::I32Const(0));
        self.op(I::LocalGet(base));
        self.op(I::I32Store(memarg(layout::DATA_STACK_PTR, ValType::I32)));
        for (i, vt) in vts.into_iter().enumerate() {
            self.op(I::LocalGet(base));
            match vt {
                ValType::Ref(r) => {
                    // The slot holds this value's index in `wack.refs`.
                    self.op(I::I32Load(memarg(
                        layout::STACK_SLOT * i as u32,
                        ValType::I32,
                    )));
                    self.op(I::TableGet(layout::REFS_TABLE));
                    self.op(I::RefCastNullable(r.heap_type));
                }
                _ => self.op(load_vt(vt, layout::STACK_SLOT * i as u32)),
            }
        }
        base
    }

    /// Push a line's outputs from the wasm stack onto the memory data stack.
    fn line_epilogue(&mut self, base: u32, outputs: &[Ty]) {
        let vts = self.lower_all(outputs);
        let size = (layout::STACK_SLOT * vts.len() as u32) as i32;
        let vals: Vec<u32> = self.stash(outputs, None).into_iter().flatten().collect();
        self.op(I::LocalGet(base));
        self.op(I::I32Const(size));
        self.op(I::I32Add);
        self.op(I::I32Const(layout::DATA_STACK_END as i32));
        self.op(I::I32GtU);
        self.op(I::If(BlockType::Empty));
        self.trap("data stack overflow");
        self.op(I::End);
        let mut ix = None;
        for (j, (vt, v)) in vts.into_iter().zip(vals).enumerate() {
            let off = layout::STACK_SLOT * j as u32;
            if let ValType::Ref(_) = vt {
                // A reference goes in `wack.refs` at the slot's own index,
                // and the slot holds that index.
                let ix = *ix.get_or_insert_with(|| self.new_local(ValType::I32));
                self.op(I::LocalGet(base));
                self.op(I::I32Const(layout::DATA_STACK_BASE as i32));
                self.op(I::I32Sub);
                self.op(I::I32Const(layout::STACK_SLOT.trailing_zeros() as i32));
                self.op(I::I32ShrU);
                self.op(I::I32Const(j as i32));
                self.op(I::I32Add);
                self.op(I::LocalSet(ix));
                self.op(I::LocalGet(ix));
                self.op(I::LocalGet(v));
                self.op(I::TableSet(layout::REFS_TABLE));
                self.op(I::LocalGet(base));
                self.op(I::LocalGet(ix));
                self.op(I::I32Store(memarg(off, ValType::I32)));
            } else {
                self.op(I::LocalGet(base));
                self.op(I::LocalGet(v));
                self.op(store_vt(vt, off));
            }
        }
        self.op(I::I32Const(0));
        self.op(I::LocalGet(base));
        self.op(I::I32Const(size));
        self.op(I::I32Add);
        self.op(I::I32Store(memarg(layout::DATA_STACK_PTR, ValType::I32)));
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

    /// In the emitting pass, a concrete generic type is registered on first
    /// use; the checking pass registers nothing (placeholders stand in).
    fn lower(&mut self, t: &Ty) -> Vec<ValType> {
        let t = self.subst.resolve(t);
        if self.emit {
            self.ctx.register_types(std::slice::from_ref(&t));
        }
        t.lower(&self.ctx.struct_types)
    }

    fn lower_all(&mut self, tys: &[Ty]) -> Vec<ValType> {
        let tys = self.subst.resolve_all(tys);
        if self.emit {
            self.ctx.register_types(&tys);
        }
        lower_all(&tys, &self.ctx.struct_types)
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
        let n = ta.counts.entry(vt).or_insert(0);
        let k = *n;
        *n += 1;
        if let Some(&i) = self.temps.get(&(vt, k)) {
            return i;
        }
        let i = self.new_local(vt);
        self.temps.insert((vt, k), i);
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

    /// The GC array type index when `elem` is a struct or a function value:
    /// an `array S` is then a view `( ref start len )` over a WasmGC array.
    /// `None` for linear arrays (and for unresolved elements in the checking
    /// pass, whose code is discarded).
    fn gc_array(&mut self, elem: &Ty) -> Option<u32> {
        match self.subst.resolve(elem) {
            t @ Ty::Struct(..) => {
                if self.emit {
                    self.ctx.register_type(&t);
                }
                self.ctx.struct_types.get(&t.to_string()).map(|&(_, a)| a)
            }
            t @ Ty::Quot(_) if mentions_quot(&t) => {
                if self.emit {
                    self.ctx.closure_type();
                }
                self.ctx.struct_types.get(CLOSURE).map(|&(_, a)| a)
            }
            _ => None,
        }
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

    /// `value v1: [ ... ] v2: [ ... ] else: [ ... ] match`: an if/else chain
    /// of `ref.test` on the variants named, in the order written; `else:`
    /// (or `unreachable` when every variant is named) is the last `else`.
    fn match_(&mut self, arms: &[Arm], loc: &Location) -> Result<Flow, Diagnostic> {
        let top = self.pop_any("match", 1, loc)?.remove(0);
        let ut = self.subst.resolve(&top);
        let (u, args) = match &ut {
            Ty::Struct(n, args) if self.ctx.union_by_name.contains_key(n) => {
                (n.clone(), args.clone())
            }
            Ty::Var(_) => {
                return Err(self.err(
                    codes::E_AMBIGUOUS_TYPE,
                    "the value matched is not known; write the effect or add a stack assertion",
                    loc,
                ))
            }
            t => {
                return Err(self
                    .err(
                        codes::E_TYPE_MISMATCH,
                        format!(
                            "`match` needs a union value on top of the stack but found ( {t} )"
                        ),
                        loc,
                    )
                    .with_stacks(vec!["union".into()], vec![t.to_string()]))
            }
        };
        let def = self.ctx.unions[self.ctx.union_by_name[&u]].clone();
        let all: Vec<&str> = def.variants.iter().map(|(v, _)| v.as_str()).collect();
        let listed = format!(
            "`{u}` has {}",
            all.iter()
                .map(|v| format!("`{v}:`"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let mut seen: Vec<&str> = Vec::new();
        for a in arms {
            let l = a.label.as_str();
            if l != "else" && !all.contains(&l) {
                return Err(self.err(
                    codes::E_MATCH_ARM,
                    format!("`{l}:` is not a variant of `{u}`; {listed}"),
                    &a.loc,
                ));
            }
            if seen.contains(&l) {
                return Err(self.err(
                    codes::E_MATCH_ARM,
                    format!("`{l}:` appears twice in this `match`; {listed}"),
                    &a.loc,
                ));
            }
            seen.push(l);
        }
        let has_else = seen.contains(&"else");
        let missing: Vec<String> = all
            .iter()
            .filter(|v| !seen.contains(v))
            .map(|v| v.to_string())
            .collect();
        if has_else && missing.is_empty() {
            let at = &arms.iter().find(|a| a.label == "else").unwrap().loc;
            return Err(self.err(
                codes::E_MATCH_ARM,
                format!("`else:` can never run: every variant of `{u}` is named"),
                at,
            ));
        }
        if !has_else && !missing.is_empty() {
            return Err(self
                .err(
                    codes::E_MATCH_MISSING,
                    format!(
                        "this `match` on `{u}` has no arm for {}; add {} or `else:`",
                        missing
                            .iter()
                            .map(|v| format!("`{v}`"))
                            .collect::<Vec<_>>()
                            .join(", "),
                        missing
                            .iter()
                            .map(|v| format!("`{v}: [ ... ]`"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                    loc,
                )
                .with_stacks(missing, arms.iter().map(|a| a.label.clone()).collect()));
        }
        let map: HashMap<String, Ty> = def
            .params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect();
        let s = self.stack.clone();
        if self.emit {
            self.ctx.register_type(&ut);
        }
        let base = self.ctx.struct_types.get(&ut.to_string()).map(|&(i, _)| i);
        let v = match (self.emit, base) {
            (true, Some(i)) => self.new_local(ref_ty(i)),
            _ => self.new_local(ValType::I32),
        };
        let hs = self.transformed
            && arms
                .iter()
                .any(|a| self.body_has_site(&a.body, &mut HashSet::new()));
        // Rewinding, `v` is restored and the value matched is a dummy.
        let mut with_u = s.clone();
        with_u.push(ut.clone());
        self.segment(hs, with_u, s.clone(), |w| {
            w.op(I::LocalSet(v));
            w.op(I::LocalGet(v));
            w.op(I::RefAsNonNull);
            w.op(I::Drop);
        });
        // Walk each arm in order, variants first then `else:`.
        let mut order: Vec<&Arm> = arms.iter().filter(|a| a.label != "else").collect();
        order.extend(arms.iter().filter(|a| a.label == "else"));
        let mut walked: Vec<(WalkedArm, &Arm)> = Vec::new();
        let named = order.len() - has_else as usize;
        for (i, a) in order.iter().enumerate() {
            let lo = self.sites.len() as u32;
            self.stack = s.clone();
            let mut pre: Vec<I<'static>> = Vec::new();
            if a.label == "else" {
                self.stack.push(ut.clone());
                pre.push(I::LocalGet(v));
            } else {
                let k = all.iter().position(|x| *x == a.label).unwrap();
                let vi = base.map(|i| i + 1 + k as u32).unwrap_or(0);
                let mut w = 0;
                for (_, ft) in &def.variants[k].1 {
                    let ft = ft.substitute(&map);
                    for j in 0..ft.width() {
                        pre.extend([
                            I::LocalGet(v),
                            I::RefCastNonNull(HeapType::Concrete(vi)),
                            I::StructGet {
                                struct_type_index: vi,
                                field_index: w + j,
                            },
                        ]);
                    }
                    w += ft.width();
                    self.stack.push(ft);
                }
            }
            // Arm i sits inside i + 1 nested `if`s; `else:` inside all of them.
            let nest = (i + 1).min(named) as u32;
            self.depth += nest;
            let r = self.seq_into(&a.body);
            self.depth -= nest;
            let (flow, code) = r?;
            pre.extend(code);
            let range = (lo, self.sites.len() as u32 - lo);
            walked.push(((flow, std::mem::take(&mut self.stack), pre, range), a));
        }
        let mut result: Option<(Vec<Ty>, &Arm)> = None;
        for ((flow, st, _, _), a) in &walked {
            if *flow != Flow::Normal {
                continue;
            }
            match &result {
                None => result = Some((st.clone(), a)),
                Some((r, first)) => {
                    if !self.subst.unify_all(r, st) {
                        let (x, y) = (self.subst.resolve_all(r), self.subst.resolve_all(st));
                        return Err(self
                            .err(
                                codes::E_BRANCH_MISMATCH,
                                format!(
                                    "arms of `match` disagree: `{}:` leaves {}, `{}:` leaves {}",
                                    first.label,
                                    fmt_stack(&x),
                                    a.label,
                                    fmt_stack(&y)
                                ),
                                &a.loc,
                            )
                            .with_stacks(names(&x), names(&y)));
                    }
                }
            }
        }
        let (flow, result) = match result {
            Some((r, _)) => (Flow::Normal, r),
            None => (Flow::Diverged, s.clone()),
        };
        let bt = self.block_type(&s, &result);
        let mut ends = 0;
        for ((_, _, code, (lo, len)), a) in walked {
            if a.label == "else" {
                self.code.extend(code);
                continue;
            }
            let k = all.iter().position(|x| *x == a.label).unwrap();
            let vi = base.map(|i| i + 1 + k as u32).unwrap_or(0);
            self.op(I::LocalGet(v));
            self.op(I::RefTestNonNull(HeapType::Concrete(vi)));
            if hs {
                self.rewind_cond(lo, len, true);
            }
            self.op(I::If(bt));
            self.code.extend(code);
            self.op(I::Else);
            ends += 1;
        }
        if !has_else {
            self.no_arm(hs, &s, &result);
        }
        for _ in 0..ends {
            self.op(I::End);
        }
        self.stack = result;
        Ok(flow)
    }

    /// `a recv: [ ... ] b recv: [ ... ] alt`: wait for the first channel with
    /// a value (or closed and drained) through `OP_ALT`, then run its arm with
    /// `option T`, from `chan.taken`.
    fn alt_(&mut self, arms: &[AltArm]) -> Result<Flow, Diagnostic> {
        // Each arm's channel, into a local.
        let mut chans: Vec<(Ty, u32, u32)> = Vec::new();
        let tr = self.transformed;
        for a in arms {
            // Rewinding, a site-free channel expression is skipped.
            let flow = if tr && !self.has_site(&a.chan, &mut HashSet::new()) {
                self.skip_in_rewind(|w| w.node(&a.chan))?
            } else {
                self.node(&a.chan)?
            };
            if flow != Flow::Normal {
                return Err(self.err(
                    codes::E_UNREACHABLE,
                    "an `alt` channel cannot diverge",
                    &a.chan.loc,
                ));
            }
            let top = self.pop_any("alt", 1, &a.chan.loc)?.remove(0);
            let v = self.subst.fresh();
            let want = Ty::Struct("chan".into(), vec![v.clone()]);
            if !self.subst.unify(&top, &want) {
                let t = self.subst.resolve(&top);
                return Err(self
                    .err(
                        codes::E_TYPE_MISMATCH,
                        format!("an `alt` arm needs a channel before `recv:` but found ( {t} )"),
                        &a.loc,
                    )
                    .with_stacks(vec!["chan T".into()], vec![t.to_string()]));
            }
            let ct = self.subst.resolve(&want);
            if self.emit && ct.has_var() {
                return Err(self.err(
                    codes::E_AMBIGUOUS_TYPE,
                    "the type of this `alt` channel is not known; write the effect or add a stack assertion",
                    &a.loc,
                ));
            }
            let vt = self.lower(&ct)[0];
            let si = self
                .ctx
                .struct_types
                .get(&ct.to_string())
                .map_or(0, |&(i, _)| i);
            let local = self.new_local(vt);
            let stack = self.stack.clone();
            let with_c = [stack.clone(), vec![ct.clone()]].concat();
            self.segment(tr, with_c, stack, |w| w.op(I::LocalSet(local)));
            chans.push((ct, local, si));
        }
        // Their ids, in memory, for the host.
        let n = arms.len() as u32;
        let buf = self.new_local(ValType::I32);
        let r = self.new_local(ValType::I32);
        let s = self.stack.clone();
        let with_args = [s.clone(), vec![Ty::I32; 3]].concat();
        self.segment(tr, s.clone(), with_args, |w| {
            w.op(I::I32Const((4 * n) as i32));
            w.op(I::Call(FN_ALLOC));
            w.op(I::LocalSet(buf));
            for (i, (_, local, si)) in chans.iter().enumerate() {
                w.op(I::LocalGet(buf));
                w.op(I::LocalGet(*local));
                w.op(I::StructGet {
                    struct_type_index: *si,
                    field_index: 0,
                });
                w.op(I::I32Store(memarg(4 * i as u32, ValType::I32)));
            }
            w.op(I::LocalGet(buf));
            w.op(I::I32Const(n as i32));
            w.op(I::I32Const(0));
        });
        let ring = vec![I::I32Const(layout::OP_ALT), I::Call(FN_RING)];
        self.site_call(
            vec![ValType::I32; 3],
            vec![ValType::I32],
            ring.clone(),
            ring,
            vec![ValType::I32; 3],
        );
        // Rewinding to a later site, the ring's dummy result never reaches `r`.
        let with_r = [s.clone(), vec![Ty::I32]].concat();
        self.segment(tr, with_r, s.clone(), |w| w.op(I::LocalSet(r)));
        // Walk each arm: it receives `option T` from `chan.taken`.
        let taken_site = tr && self.name_is_site("chan.taken", &HashSet::new());
        let mut walked: Vec<WalkedArm> = Vec::new();
        for (i, (a, (ct, local, _))) in arms.iter().zip(&chans).enumerate() {
            let saved = std::mem::take(&mut self.code);
            let lo = self.sites.len() as u32;
            // Arm i sits inside i + 1 nested `if`s.
            let nest = i as u32 + 1;
            self.depth += nest;
            let pushes = |w: &mut Self| {
                w.op(I::LocalGet(*local));
                w.op(I::LocalGet(r));
                w.op(I::I32Const(1));
                w.op(I::I32And);
            };
            let with_taken = [s.clone(), vec![ct.clone(), Ty::I32]].concat();
            let taken = if !tr || taken_site {
                self.segment(tr, s.clone(), with_taken.clone(), pushes);
                self.stack = with_taken;
                self.name_ref("chan.taken", &a.loc)
            } else {
                // Not a site: `chan.taken` is skipped with its pushes.
                self.stack = s.clone();
                self.skip_in_rewind(|w| {
                    pushes(w);
                    w.stack = with_taken;
                    w.name_ref("chan.taken", &a.loc)
                })
            };
            let flow = taken.and_then(|_| self.seq(&a.body));
            self.depth -= nest;
            let code = std::mem::replace(&mut self.code, saved);
            let range = (lo, self.sites.len() as u32 - lo);
            walked.push((flow?, std::mem::take(&mut self.stack), code, range));
        }
        let mut result: Option<(Vec<Ty>, usize)> = None;
        for (k, (flow, st, _, _)) in walked.iter().enumerate() {
            if *flow != Flow::Normal {
                continue;
            }
            match &result {
                None => result = Some((st.clone(), k)),
                Some((rs, first)) => {
                    if !self.subst.unify_all(rs, st) {
                        let (x, y) = (self.subst.resolve_all(rs), self.subst.resolve_all(st));
                        return Err(self
                            .err(
                                codes::E_BRANCH_MISMATCH,
                                format!(
                                    "arms of `alt` disagree: arm {} leaves {}, arm {} leaves {}",
                                    first + 1,
                                    fmt_stack(&x),
                                    k + 1,
                                    fmt_stack(&y)
                                ),
                                &arms[k].loc,
                            )
                            .with_stacks(names(&x), names(&y)));
                    }
                }
            }
        }
        let (flow, result) = match result {
            Some((rs, _)) => (Flow::Normal, rs),
            None => (Flow::Diverged, s.clone()),
        };
        let bt = self.block_type(&s, &result);
        for (i, (_, _, code, (lo, len))) in walked.into_iter().enumerate() {
            self.op(I::LocalGet(r));
            self.op(I::I32Const(1));
            self.op(I::I32ShrU);
            self.op(I::I32Const(i as i32));
            self.op(I::I32Eq);
            if tr {
                self.rewind_cond(lo, len, true);
            }
            self.op(I::If(bt));
            self.code.extend(code);
            self.op(I::Else);
        }
        self.no_arm(tr, &s, &result);
        for _ in 0..n {
            self.op(I::End);
        }
        self.stack = result;
        Ok(flow)
    }

    /// Emit `eq` or `hash` on values of the concrete type `t`: inline for
    /// numbers and function values, a call to a generated word otherwise.
    fn hash_eq(&mut self, op: Op, t: &Ty, loc: &Location) -> Result<(), Diagnostic> {
        let mut ta = TempAlloc::default();
        // Turn a float on top into its bits.
        let bits = |w: &mut Self, t: &Ty| match t {
            Ty::F32 => w.op(I::I32ReinterpretF32),
            Ty::F64 => w.op(I::I64ReinterpretF64),
            _ => {}
        };
        let wide = matches!(t, Ty::I64 | Ty::F64);
        match t {
            Ty::I32 | Ty::Quot(_) | Ty::I64 | Ty::F32 | Ty::F64 => {
                // A function value: equal when the same closure, hashed by
                // its slot.
                if let Ty::Quot(_) = t {
                    if op == Op::Eq {
                        self.op(I::RefEq);
                        return Ok(());
                    }
                    let c = self.ctx.closure_type();
                    self.op(I::StructGet {
                        struct_type_index: c,
                        field_index: 0,
                    });
                }
                if op == Op::Eq {
                    if matches!(t, Ty::F32 | Ty::F64) {
                        let vt = if wide { ValType::I64 } else { ValType::I32 };
                        bits(self, t);
                        let b = self.temp(&mut ta, vt);
                        self.op(I::LocalSet(b));
                        bits(self, t);
                        self.op(I::LocalGet(b));
                    }
                    self.op(if wide { I::I64Eq } else { I::I32Eq });
                    return Ok(());
                }
                bits(self, t);
                if wide {
                    // Fold the high half into the low.
                    let x = self.temp(&mut ta, ValType::I64);
                    self.op(I::LocalTee(x));
                    self.op(I::I32WrapI64);
                    self.op(I::LocalGet(x));
                    self.op(I::I64Const(32));
                    self.op(I::I64ShrU);
                    self.op(I::I32WrapI64);
                    self.op(I::I32Xor);
                }
                // A multiply-and-xorshift mix.
                self.op(I::I32Const(0x9E37_79B1_u32 as i32));
                self.op(I::I32Mul);
                let h = self.temp(&mut ta, ValType::I32);
                self.op(I::LocalTee(h));
                self.op(I::LocalGet(h));
                self.op(I::I32Const(16));
                self.op(I::I32ShrU);
                self.op(I::I32Xor);
                Ok(())
            }
            _ => {
                let id = self.ctx.hash_eq_word(op, t, loc)?;
                let e = self.ctx.words[id].effect.clone();
                self.word_call(id, &e);
                Ok(())
            }
        }
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
        if self.transformed {
            return self.seq_transformed(body);
        }
        self.seq_nodes(body, Flow::Normal)
    }

    /// Walk `nodes` in order after code that ended with `flow`.
    fn seq_nodes(&mut self, nodes: &[Node], mut flow: Flow) -> Result<Flow, Diagnostic> {
        for node in nodes {
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

    /// `seq` in a transformed function (design step 3): each maximal run of
    /// nodes without a suspendable site is skipped in rewind mode.
    fn seq_transformed(&mut self, body: &Body) -> Result<Flow, Diagnostic> {
        let mut bound = HashSet::new();
        let sites = self.site_flags(body, &mut bound);
        let mut flow = Flow::Normal;
        let mut i = 0;
        while i < body.len() {
            if sites[i] {
                flow = self.seq_nodes(&body[i..i + 1], flow)?;
                i += 1;
                continue;
            }
            let j = (i..body.len()).find(|&j| sites[j]).unwrap_or(body.len());
            let run = &body[i..j];
            if flow == Flow::Diverged {
                // Reported by `seq_nodes` without wrapping.
                return self.seq_nodes(run, flow);
            }
            flow = self.skip_in_rewind(|w| w.seq_nodes(run, Flow::Normal))?;
            i = j;
        }
        Ok(flow)
    }

    /// Whether `node` may hold a suspendable call site, before it is walked
    /// (a superset is safe: it only costs code). `bound` collects the
    /// locals bound so far in the scan.
    fn has_site(&self, node: &Node, bound: &mut HashSet<String>) -> bool {
        match &node.kind {
            NodeKind::Name(n) => self.name_is_site(n, bound),
            NodeKind::Bind { name, .. } => {
                bound.insert(name.clone());
                false
            }
            NodeKind::Alt(_) => true,
            NodeKind::If(a, b) | NodeKind::While(a, b) | NodeKind::Until(a, b) => {
                let a = self.body_has_site(a, bound);
                self.body_has_site(b, bound) || a
            }
            NodeKind::When(b)
            | NodeKind::Unless(b)
            | NodeKind::Times(b)
            | NodeKind::Each(b)
            | NodeKind::Map(b)
            | NodeKind::Filter(b)
            | NodeKind::Fold(b) => self.body_has_site(b, bound),
            NodeKind::Match(arms) => {
                // Scan every arm: each may bind locals.
                let mut any = false;
                for a in arms {
                    any |= self.body_has_site(&a.body, bound);
                }
                any
            }
            NodeKind::Lit(_)
            | NodeKind::Tick(_)
            | NodeKind::Quote { .. }
            | NodeKind::Assert(_)
            | NodeKind::Leave => false,
        }
    }

    fn body_has_site(&self, body: &Body, bound: &mut HashSet<String>) -> bool {
        self.site_flags(body, bound).contains(&true)
    }

    /// `has_site` of each node of `body`. A parking opcode literal directly
    /// before `ring.submit` belongs to that site: emitted with it, so the
    /// call sees its opcode.
    fn site_flags(&self, body: &Body, bound: &mut HashSet<String>) -> Vec<bool> {
        let mut flags: Vec<bool> = body.iter().map(|n| self.has_site(n, bound)).collect();
        for i in 1..body.len() {
            if matches!(&body[i].kind, NodeKind::Name(n) if n == "ring.submit")
                && matches!(
                    body[i - 1].kind,
                    NodeKind::Lit(Lit::I32(
                        layout::OP_CHAN_SEND
                            | layout::OP_CHAN_RECV
                            | layout::OP_ALT
                            | layout::OP_SLEEP
                    ))
                )
            {
                flags[i - 1] = true;
                flags[i] = true;
            }
        }
        flags
    }

    fn name_is_site(&self, n: &str, bound: &HashSet<String>) -> bool {
        if n == "call" {
            return true;
        }
        let base = n.strip_suffix('!').unwrap_or(n);
        if bound.contains(base)
            || self.locals.iter().any(|l| l.name == base)
            || self.capture(base).is_some()
            || prims::is_builtin(n)
        {
            return false;
        }
        match &self.ctx.unwind {
            Unwind::Off => false,
            Unwind::All => true,
            Unwind::Only(s) => s.contains(n),
        }
    }

    /// Emit a straight-line segment of a construct that holds a site: when
    /// `on`, it is skipped in rewind mode, its stack going from `before` to
    /// `after` (dummies). The checker stack is left as it was.
    fn segment(&mut self, on: bool, before: Vec<Ty>, after: Vec<Ty>, emit: impl FnOnce(&mut Self)) {
        if !on {
            emit(self);
            return;
        }
        let saved = std::mem::replace(&mut self.stack, before);
        self.skip_in_rewind(|w| {
            emit(w);
            w.stack = after;
            Ok(Flow::Normal)
        })
        .expect("a segment reports nothing");
        self.stack = saved;
    }

    /// Design step 4: rewinding, replace the condition on top of the stack
    /// with whether the resume site is (`inside`) or is not in
    /// `[lo, lo + len)`. Returns where `len` is, for `patch_range`.
    fn rewind_cond(&mut self, lo: u32, len: u32, inside: bool) -> usize {
        self.op(I::LocalGet(self.resume_local));
        self.op(I::I32Const(lo as i32));
        self.op(I::I32Sub);
        let at = self.code.len();
        self.op(I::I32Const(len as i32));
        self.op(I::I32LtU);
        if !inside {
            self.op(I::I32Eqz);
        }
        self.op(I::I32Const(0));
        self.op(I::I32Load(memarg(layout::UNWIND_MODE, ValType::I32)));
        self.op(I::I32Const(layout::REWINDING));
        self.op(I::I32Ne);
        self.op(I::Select);
        at
    }

    /// Where no arm of a `match` or `alt` was taken: unreachable, except
    /// (`rewinding`) when rewinding to a site after the construct, which
    /// passes dummies through.
    fn no_arm(&mut self, rewinding: bool, s: &[Ty], result: &[Ty]) {
        if !rewinding {
            self.op(I::Unreachable);
            return;
        }
        let bt = self.block_type(s, result);
        self.code.extend(mode_is(layout::REWINDING));
        self.op(I::If(bt));
        for _ in self.lower_all(s) {
            self.op(I::Drop);
        }
        for vt in self.lower_all(result) {
            self.op(dummy(vt));
        }
        self.op(I::Else);
        self.op(I::Unreachable);
        self.op(I::End);
    }

    /// The range of a `rewind_cond` at `at` ends at the sites walked so far.
    fn patch_range(&mut self, at: usize, lo: u32) {
        self.code[at] = I::I32Const((self.sites.len() as u32 - lo) as i32);
    }

    /// Whether `node` holds a suspendable site in a transformed function.
    fn node_has_site(&self, node: &Node) -> bool {
        self.transformed && self.has_site(node, &mut HashSet::new())
    }

    fn body_site(&self, b: &Body) -> bool {
        self.transformed && self.body_has_site(b, &mut HashSet::new())
    }

    /// Emit `walk`'s code so that it runs only when not rewinding; rewinding,
    /// its inputs are dropped and dummies of its outputs pushed instead. The
    /// block type is filled in once the walk has left its stack.
    fn skip_in_rewind(
        &mut self,
        walk: impl FnOnce(&mut Self) -> Result<Flow, Diagnostic>,
    ) -> Result<Flow, Diagnostic> {
        let before = self.stack.clone();
        self.op(I::I32Const(0));
        self.op(I::I32Load(memarg(layout::UNWIND_MODE, ValType::I32)));
        self.op(I::I32Const(layout::REWINDING));
        self.op(I::I32Ne);
        let at = self.code.len();
        self.op(I::If(BlockType::Empty));
        self.open_label();
        let flow = walk(self)?;
        let after = if flow == Flow::Diverged {
            before.clone()
        } else {
            self.stack.clone()
        };
        self.code[at] = I::If(self.block_type(&before, &after));
        self.op(I::Else);
        if flow == Flow::Diverged {
            self.op(I::Unreachable);
        } else {
            for _ in self.lower_all(&before) {
                self.op(I::Drop);
            }
            for vt in self.lower_all(&after) {
                self.op(dummy(vt));
            }
        }
        self.close_label();
        if flow == Flow::Diverged {
            self.op(I::Unreachable);
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
                let Some(id) = self.ctx.lookup(n) else {
                    return Err(self.undefined(n, loc));
                };
                if self.ctx.words[id].generic.is_some() {
                    let (e, vars) = self.fresh_instance(id);
                    self.stack.push(Ty::Quot(Box::new(e)));
                    if self.emit {
                        let inst = self.instance(id, &format!("'{n}"), &vars, loc)?;
                        self.tick(inst);
                    }
                    return Ok(Flow::Normal);
                }
                let e = self.ctx.words[id].effect.clone();
                if self.emit {
                    self.tick(id);
                }
                self.stack.push(Ty::Quot(Box::new(e)));
                Ok(Flow::Normal)
            }
            NodeKind::Quote { body, effect } => {
                let mut visible = self.outer_locals.clone();
                for l in &self.locals {
                    visible.push(Capture {
                        name: l.name.clone(),
                        ty: self.subst.resolve(&l.ty),
                        mutable: l.mutable,
                    });
                }
                let qname = format!("[quote {}:{}:{}]", self.name, loc.line, loc.column);
                let annotated = effect.as_ref().map(|e| {
                    if self.ctx.type_params.is_empty() {
                        e.clone()
                    } else {
                        e.substitute(&self.ctx.type_params)
                    }
                });
                // Checked in both passes alike, so variables number alike.
                let (effect, captures) = match annotated {
                    Some(e) => {
                        self.ctx.check_types(&e.inputs, loc, None)?;
                        self.ctx.check_types(&e.outputs, loc, None)?;
                        let mut w =
                            Walker::new(self.ctx, &qname, false, self.subst.clone(), &visible);
                        w.run(&Mode::Closure(&e), body, loc)?;
                        self.subst = w.subst;
                        (e, w.captures)
                    }
                    None => self.infer_quote(&qname, body, loc, &visible)?,
                };
                let ty = Ty::Quot(Box::new(effect.clone()));
                if self.emit {
                    let resolved = self.subst.resolve(&ty);
                    if resolved.has_var() {
                        return Err(Diagnostic::error(
                            codes::E_AMBIGUOUS_TYPE,
                            format!("the type `{resolved}` of this quotation is not fully known; write its effect directly after `[`, e.g. `[ ( i32 -- i32 ) ... ]`"),
                            loc.clone(),
                        ));
                    }
                    let Ty::Quot(effect) = resolved else {
                        unreachable!()
                    };
                    let out = compile_body(
                        self.ctx,
                        &qname,
                        Mode::Closure(&effect),
                        body,
                        loc,
                        &visible,
                    )?;
                    debug_assert_eq!(captures, out.captures);
                    let id = self.ctx.add_word(Word {
                        name: qname,
                        effect: out.effect,
                        body: Some(out.compiled),
                        failed: false,
                        export: false,
                        raw: false,
                        origin: Origin::User,
                        kind: WordKind::Quote,
                        loc: loc.clone(),
                        callees: out.callees,
                        inferred: false,
                        generic: None,
                        instance_of: None,
                        generated: None,
                    });
                    self.callees.push((id, EdgeKind::AddressTaken));
                    self.op(I::I32Const(id as i32));
                    for n in &out.captures {
                        self.read_outer(n, loc)?;
                    }
                    let t = match out.env_type {
                        Some(t) => t,
                        None => self.ctx.closure_type(),
                    };
                    self.op(I::StructNew(t));
                } else {
                    // Record what an enclosing quotation must capture for it.
                    for n in &captures {
                        self.read_outer(n, loc)?;
                    }
                    self.op(I::I32Const(0));
                }
                self.stack.push(ty);
                Ok(Flow::Normal)
            }
            NodeKind::Assert(tys) => {
                let subst: Vec<Ty>;
                let tys = if self.ctx.type_params.is_empty() {
                    tys
                } else {
                    subst = tys
                        .iter()
                        .map(|t| t.substitute(&self.ctx.type_params))
                        .collect();
                    &subst
                };
                self.ctx.check_types(tys, loc, None)?;
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
                let hs = self.node_has_site(node);
                self.open_label();
                let lo = self.sites.len() as u32;
                let (ft, ct) = self.seq_into(t)?;
                let mid = self.sites.len() as u32;
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
                if hs {
                    // Rewinding, take the branch holding the resume site.
                    self.rewind_cond(lo, mid - lo, true);
                }
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
                let lo = self.sites.len() as u32;
                let at = self
                    .node_has_site(node)
                    .then(|| self.rewind_cond(lo, 0, what == "when"));
                if what == "unless" {
                    self.op(I::I32Eqz);
                }
                let bt = self.block_type(&s, &s);
                self.op(I::If(bt));
                self.open_label();
                let flow = self.seq(b)?;
                self.expect_identity(flow, &s, what, "body", loc)?;
                if let Some(at) = at {
                    self.patch_range(at, lo);
                }
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
                let hs = self.node_has_site(node);
                let fc = self.seq(c)?;
                if fc == Flow::Normal {
                    let mut want = s.clone();
                    want.push(Ty::I32);
                    self.expect_shape(&want, "while", "condition", "plus one i32", loc)?;
                    self.stack.pop();
                }
                // Rewinding (to the body: the condition's sites are behind),
                // enter the loop only when the resume site is in the body.
                let mid = self.sites.len() as u32;
                let at = hs.then(|| self.rewind_cond(mid, 0, true));
                self.op(I::I32Eqz);
                self.op(I::BrIf(self.rel(exit)));
                let fb = self.seq(b)?;
                self.expect_identity(fb, &s, "while", "body", loc)?;
                if let Some(at) = at {
                    self.patch_range(at, mid);
                }
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
                let hs = self.node_has_site(node);
                let lo = self.sites.len() as u32;
                let fb = self.seq(b)?;
                self.expect_identity(fb, &s, "until", "body", loc)?;
                let fc = self.seq(c)?;
                if fc == Flow::Normal {
                    let mut want = s.clone();
                    want.push(Ty::I32);
                    self.expect_shape(&want, "until", "condition", "plus one i32", loc)?;
                    self.stack.pop();
                }
                // Still rewinding here, the resume site lies after the loop:
                // leave it rather than run it again.
                if hs {
                    let len = self.sites.len() as u32 - lo;
                    self.rewind_cond(lo, len, false);
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
                let hs = self.node_has_site(node);
                let mut before = s.clone();
                before.push(Ty::I32);
                self.segment(hs, before, s.clone(), |w| {
                    w.op(I::LocalSet(n));
                    w.op(I::I32Const(0));
                    w.op(I::LocalSet(i));
                });
                let bt = self.block_type(&s, &s);
                self.op(I::Block(bt));
                let exit = self.open_label();
                self.op(I::Loop(bt));
                let top = self.open_label();
                self.op(I::LocalGet(i));
                self.op(I::LocalGet(n));
                self.op(I::I32GeS);
                let lo = self.sites.len() as u32;
                let at = hs.then(|| self.rewind_cond(lo, 0, false));
                self.op(I::BrIf(self.rel(exit)));
                self.op(I::LocalGet(i));
                self.stack.push(Ty::I32);
                self.loops.push(LoopCtx {
                    exit: Some(s.clone()),
                    level: exit,
                });
                let fb = self.seq(b)?;
                self.expect_identity(fb, &s, "times", "body (which receives the index)", loc)?;
                if let Some(at) = at {
                    self.patch_range(at, lo);
                }
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
            NodeKind::Match(arms) => self.match_(arms, loc),
            NodeKind::Alt(arms) => self.alt_(arms),
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
    fn load_elem(&mut self, t: &Ty, arr: &ArrLocals, i: u32, ta: &mut TempAlloc) {
        let addr = match *arr {
            ArrLocals::Linear(addr) => addr,
            ArrLocals::Gc { arr, start, ti } => {
                self.gc_index(arr, start, i);
                self.op(I::ArrayGet(ti));
                return;
            }
        };
        let size = t.elem_size() as i32;
        let ea = self.temp(ta, ValType::I32);
        self.op(I::LocalGet(addr));
        self.op(I::LocalGet(i));
        self.op(I::I32Const(size));
        self.op(I::I32Mul);
        self.op(I::I32Add);
        self.op(I::LocalSet(ea));
        let mut off = 0;
        for vt in self.lower(t) {
            self.op(I::LocalGet(ea));
            self.op(load_vt(vt, off));
            off += vt_size(vt);
        }
    }

    /// Emit: store value locals `val` as element `i` of the array at `addr`.
    fn store_elem(&mut self, t: &Ty, arr: &ArrLocals, i: u32, val: &[u32], ta: &mut TempAlloc) {
        let addr = match *arr {
            ArrLocals::Linear(addr) => addr,
            ArrLocals::Gc { arr, start, ti } => {
                self.gc_index(arr, start, i);
                self.op(I::LocalGet(val[0]));
                self.op(I::ArraySet(ti));
                return;
            }
        };
        let size = t.elem_size() as i32;
        let ea = self.temp(ta, ValType::I32);
        self.op(I::LocalGet(addr));
        self.op(I::LocalGet(i));
        self.op(I::I32Const(size));
        self.op(I::I32Mul);
        self.op(I::I32Add);
        self.op(I::LocalSet(ea));
        let mut off = 0;
        for (vt, &v) in self.lower(t).into_iter().zip(val) {
            self.op(I::LocalGet(ea));
            self.op(I::LocalGet(v));
            self.op(store_vt(vt, off));
            off += vt_size(vt);
        }
    }

    /// Push the GC array and the element index `start + i` (start 0 if none).
    fn gc_index(&mut self, arr: u32, start: Option<u32>, i: u32) {
        self.op(I::LocalGet(arr));
        self.op(I::LocalGet(i));
        if let Some(s) = start {
            self.op(I::LocalGet(s));
            self.op(I::I32Add);
        }
    }

    /// Where a combinator's array lives: a linear `addr`, or a struct-array
    /// view whose GC array is in a ref local. `len` is already allocated; for
    /// a view, `start` reuses the `addr` local and the GC array gets a new one.
    fn source_array(&mut self, t: &Ty, addr: u32) -> ArrLocals {
        match self.gc_array(t) {
            Some(ti) => ArrLocals::Gc {
                arr: self.new_local(ref_ty(ti)),
                start: Some(addr),
                ti,
            },
            None => ArrLocals::Linear(addr),
        }
    }

    /// Pop the array (already typed) into its locals.
    fn array_prologue(&mut self, arr: &ArrLocals, len: u32) {
        self.op(I::LocalSet(len));
        match *arr {
            ArrLocals::Linear(addr) => self.op(I::LocalSet(addr)),
            ArrLocals::Gc { arr, start, .. } => {
                self.op(I::LocalSet(start.unwrap()));
                self.op(I::LocalSet(arr));
            }
        }
    }

    /// Common loop head for collection combinators. Returns (exit level, top
    /// level). With `rewind` (the first site of a body holding sites), the
    /// counter's reset is skipped in rewind mode and the exit test leaves
    /// unless the resume site is in the body; the returned position is for
    /// `patch_range`.
    fn coll_loop_open(
        &mut self,
        s: &[Ty],
        i: u32,
        len: u32,
        rewind: Option<u32>,
    ) -> (u32, u32, Option<usize>) {
        let bt = self.block_type(s, s);
        self.segment(rewind.is_some(), s.to_vec(), s.to_vec(), |w| {
            w.op(I::I32Const(0));
            w.op(I::LocalSet(i));
        });
        self.op(I::Block(bt));
        let exit = self.open_label();
        self.op(I::Loop(bt));
        let top = self.open_label();
        self.op(I::LocalGet(i));
        self.op(I::LocalGet(len));
        self.op(I::I32GeU);
        let at = rewind.map(|lo| self.rewind_cond(lo, 0, false));
        self.op(I::BrIf(self.rel(exit)));
        (exit, top, at)
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

    /// Push a combinator's result array: `addr len`, or a view `( ref 0 len )`.
    fn push_result(&mut self, arr: &ArrLocals, len: u32) {
        match *arr {
            ArrLocals::Linear(addr) => self.op(I::LocalGet(addr)),
            ArrLocals::Gc { arr, .. } => {
                self.op(I::LocalGet(arr));
                self.op(I::I32Const(0));
            }
        }
        self.op(I::LocalGet(len));
    }

    fn each(&mut self, b: &Body, loc: &Location) -> Result<Flow, Diagnostic> {
        let t = self.pop_array("each", loc)?;
        let t = self.concrete_elem(&t, "each", loc)?;
        let s = self.stack.clone();
        let hs = self.body_site(b);
        let len = self.new_local(ValType::I32);
        let addr = self.new_local(ValType::I32);
        let i = self.new_local(ValType::I32);
        let arr = self.source_array(&t, addr);
        let mut with_t = s.clone();
        with_t.push(t.clone());
        let with_arr = [s.clone(), vec![Ty::Array(Box::new(t.clone()))]].concat();
        self.segment(hs, with_arr, s.clone(), |w| w.array_prologue(&arr, len));
        let lo = self.sites.len() as u32;
        let (exit, top, at) = self.coll_loop_open(&s, i, len, hs.then_some(lo));
        self.segment(hs, s.clone(), with_t, |w| {
            w.load_elem(&t, &arr, i, &mut TempAlloc::default())
        });
        self.stack.push(t);
        self.loops.push(LoopCtx {
            exit: Some(s.clone()),
            level: exit,
        });
        let f = self.seq(b)?;
        self.expect_identity(f, &s, "each", "body (which receives each element)", loc)?;
        if let Some(at) = at {
            self.patch_range(at, lo);
        }
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
        let src_arr = self.source_array(&t, src);
        let hs = self.body_site(b);
        // Walk the body first (into its own buffer) to learn U.
        self.depth += 2;
        self.loops.push(LoopCtx {
            exit: None,
            level: 0,
        });
        let saved = std::mem::take(&mut self.code);
        let mut ta = TempAlloc::default();
        let mut with_t = s.clone();
        with_t.push(t.clone());
        self.segment(hs, s.clone(), with_t, |w| {
            w.load_elem(&t, &src_arr, i, &mut ta)
        });
        self.stack.push(t.clone());
        let lo = self.sites.len() as u32;
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
        // Prologue: the result follows U, a fresh linear block or GC array.
        let with_arr = [s.clone(), vec![Ty::Array(Box::new(t.clone()))]].concat();
        self.segment(hs, with_arr, s.clone(), |w| w.array_prologue(&src_arr, len));
        let mut dst_arr = None;
        self.segment(hs, s.clone(), s.clone(), |w| {
            dst_arr = Some(match w.gc_array(&u) {
                Some(ti) => {
                    let r = w.new_local(ref_ty(ti));
                    w.op(I::LocalGet(len));
                    w.op(I::ArrayNewDefault(ti));
                    w.op(I::LocalSet(r));
                    ArrLocals::Gc {
                        arr: r,
                        start: None,
                        ti,
                    }
                }
                None => {
                    w.op(I::LocalGet(len));
                    w.op(I::I32Const(u.elem_size() as i32));
                    w.op(I::I32Mul);
                    w.op(I::Call(FN_ALLOC));
                    w.op(I::LocalSet(dst));
                    ArrLocals::Linear(dst)
                }
            })
        });
        let dst_arr = dst_arr.expect("set by the segment");
        let (_exit, top, at) = self.coll_loop_open(&s, i, len, hs.then_some(lo));
        if let Some(at) = at {
            self.patch_range(at, lo);
        }
        self.code.extend(body_code);
        let val = self.stash(std::slice::from_ref(&u), None).pop().unwrap();
        self.store_elem(&u, &dst_arr, i, &val, &mut TempAlloc::default());
        self.coll_loop_close(i, top);
        self.push_result(&dst_arr, len);
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
        let src_arr = self.source_array(&t, src);
        let hs = self.body_site(b);
        let with_arr = [s.clone(), vec![Ty::Array(Box::new(t.clone()))]].concat();
        self.segment(hs, with_arr, s.clone(), |w| w.array_prologue(&src_arr, len));
        let mut dst_arr = None;
        self.segment(hs, s.clone(), s.clone(), |w| {
            dst_arr = Some(match src_arr {
                ArrLocals::Gc { ti, .. } => {
                    // A view of the first `cnt` slots of a fresh GC array.
                    let r = w.new_local(ref_ty(ti));
                    w.op(I::LocalGet(len));
                    w.op(I::ArrayNewDefault(ti));
                    w.op(I::LocalSet(r));
                    ArrLocals::Gc {
                        arr: r,
                        start: None,
                        ti,
                    }
                }
                ArrLocals::Linear(_) => {
                    w.op(I::LocalGet(len));
                    w.op(I::I32Const(t.elem_size() as i32));
                    w.op(I::I32Mul);
                    w.op(I::Call(FN_ALLOC));
                    w.op(I::LocalSet(dst));
                    ArrLocals::Linear(dst)
                }
            });
            w.op(I::I32Const(0));
            w.op(I::LocalSet(cnt));
        });
        let dst_arr = dst_arr.expect("set by the segment");
        let lo = self.sites.len() as u32;
        let (_exit, top, at) = self.coll_loop_open(&s, i, len, hs.then_some(lo));
        let mut with_t = s.clone();
        with_t.push(t.clone());
        let mut val = Vec::new();
        self.segment(hs, s.clone(), with_t, |w| {
            w.load_elem(&t, &src_arr, i, &mut TempAlloc::default());
            val = w.stash(std::slice::from_ref(&t), None).pop().unwrap();
            w.unstash(&val);
        });
        self.stack.push(t.clone());
        self.loops.push(LoopCtx {
            exit: None,
            level: 0,
        });
        let f = self.seq(b)?;
        self.loops.pop();
        if let Some(at) = at {
            self.patch_range(at, lo);
        }
        if f == Flow::Normal {
            let mut want = s.clone();
            want.push(Ty::I32);
            self.expect_shape(&want, "filter", "predicate", "plus one i32", loc)?;
            self.stack.pop();
        }
        self.op(I::If(BlockType::Empty));
        self.depth += 1;
        self.store_elem(&t, &dst_arr, cnt, &val, &mut TempAlloc::default());
        self.op(I::LocalGet(cnt));
        self.op(I::I32Const(1));
        self.op(I::I32Add);
        self.op(I::LocalSet(cnt));
        self.close_label();
        self.coll_loop_close(i, top);
        self.push_result(&dst_arr, cnt);
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
        let hs = self.body_site(b);
        let before = [s.clone(), vec![Ty::Array(Box::new(t.clone())), u.clone()]].concat();
        let mut locals = None;
        self.segment(hs, before, su.clone(), |w| {
            let mut ta = TempAlloc::default();
            let acc = w
                .stash(std::slice::from_ref(&u), Some(&mut ta))
                .pop()
                .unwrap();
            let len = w.new_local(ValType::I32);
            let src = w.new_local(ValType::I32);
            let i = w.new_local(ValType::I32);
            let src_arr = w.source_array(&t, src);
            w.array_prologue(&src_arr, len);
            w.unstash(&acc);
            locals = Some((len, i, src_arr));
        });
        let (len, i, src_arr) = locals.expect("set by the segment");
        let lo = self.sites.len() as u32;
        let (exit, top, at) = self.coll_loop_open(&su, i, len, hs.then_some(lo));
        let mut with_t = su.clone();
        with_t.push(t.clone());
        self.segment(hs, su.clone(), with_t, |w| {
            w.load_elem(&t, &src_arr, i, &mut TempAlloc::default())
        });
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
        if let Some(at) = at {
            self.patch_range(at, lo);
        }
        self.loops.pop();
        self.coll_loop_close(i, top);
        self.stack = su;
        Ok(Flow::Normal)
    }

    fn undefined(&self, n: &str, loc: &Location) -> Diagnostic {
        let msg = if self.ctx.all_names.contains(n) {
            format!("`{n}` is used before it is defined; add `declare {n} ( ... -- ... )` above this use")
        } else {
            let dictionary = self
                .ctx
                .by_name
                .keys()
                .map(String::as_str)
                .chain(self.locals.iter().map(|l| l.name.as_str()));
            format!("unknown word `{n}`{}", crate::prims::suggest(n, dictionary))
        };
        self.err(codes::E_UNDEFINED, msg, loc)
    }

    /// A generic word's effect with a fresh variable for each type
    /// parameter, and those variables in parameter order.
    /// Push a closure of word `id` (`'word`): a struct holding the slot of
    /// its wrapper, a word that takes the closure parameter and calls `id`.
    /// One wrapper per word, made on first use.
    fn tick(&mut self, id: WordId) {
        let wrapper = match self.ctx.tick_wrappers.get(&id) {
            Some(&w) => w,
            None => {
                let e = self.ctx.words[id].effect.clone();
                let n = e.wasm_params(&self.ctx.struct_types).len() as u32;
                let mut code: Vec<I<'static>> = (0..n).map(I::LocalGet).collect();
                if self.ctx.indirect_calls {
                    let ti = self.ctx.func_type(&e, false);
                    code.push(I::I32Const(id as i32));
                    code.push(I::CallIndirect {
                        type_index: ti,
                        table_index: 0,
                    });
                } else {
                    code.push(I::Call(Word::func_index(id)));
                }
                let w = &self.ctx.words[id];
                let wrapper = Word {
                    name: format!("[tick {}]", w.name),
                    effect: e,
                    body: Some(Compiled {
                        locals: Vec::new(),
                        code,
                    }),
                    failed: false,
                    export: false,
                    raw: false,
                    origin: w.origin,
                    kind: WordKind::Quote,
                    loc: w.loc.clone(),
                    callees: vec![(id, EdgeKind::Call)],
                    inferred: false,
                    generic: None,
                    instance_of: None,
                    generated: None,
                };
                let wid = self.ctx.add_word(wrapper);
                self.ctx.tick_wrappers.insert(id, wid);
                wid
            }
        };
        self.callees.push((id, EdgeKind::AddressTaken));
        self.callees.push((wrapper, EdgeKind::AddressTaken));
        let c = self.ctx.closure_type();
        self.op(I::I32Const(wrapper as i32));
        self.op(I::StructNew(c));
    }

    /// The effect of an un-annotated quotation value, inferred as for an
    /// un-annotated word (the fewest inputs for which the body checks), with
    /// its type variables in this walker's substitution so that later use
    /// fixes them. A failed attempt leaves the substitution untouched, so
    /// both passes number variables alike.
    fn infer_quote(
        &mut self,
        qname: &str,
        body: &Body,
        loc: &Location,
        visible: &[Capture],
    ) -> Result<(Effect, Vec<String>), Diagnostic> {
        let mut last = None;
        for n in 0..=crate::infer::MAX_INPUTS {
            let mut w = Walker::new(self.ctx, qname, false, self.subst.clone(), visible);
            match w.run(&Mode::Infer(n), body, loc) {
                Ok(effect) => {
                    self.subst = w.subst;
                    return Ok((effect, w.captures));
                }
                Err(d) if d.code == codes::E_STACK_UNDERFLOW => last = Some(d),
                Err(d) => return Err(d),
            }
        }
        Err(last.expect("at least one attempt"))
    }

    fn fresh_instance(&mut self, id: WordId) -> (Effect, Vec<Ty>) {
        let effect = self.ctx.words[id].effect.clone();
        let params = effect.params();
        let vars: Vec<Ty> = params.iter().map(|_| self.subst.fresh()).collect();
        let map: HashMap<String, Ty> = params.into_iter().zip(vars.iter().cloned()).collect();
        (effect.substitute(&map), vars)
    }

    /// The instance of generic word `id` that `vars` now fix.
    fn instance(
        &mut self,
        id: WordId,
        n: &str,
        vars: &[Ty],
        loc: &Location,
    ) -> Result<WordId, Diagnostic> {
        let args = self.subst.resolve_all(vars);
        if args.iter().any(Ty::has_var) {
            let params = self.ctx.words[id].effect.params();
            let open: Vec<String> = params
                .iter()
                .zip(&args)
                .filter(|(_, a)| a.has_var())
                .map(|(p, _)| format!("`{p}`"))
                .collect();
            let verb = if open.len() == 1 { "is" } else { "are" };
            return Err(self.err(
                codes::E_AMBIGUOUS_TYPE,
                format!("the instantiation of `{n}` is not fixed here: {} {verb} unknown; add a stack assertion or declare the effect", open.join(", ")),
                loc,
            ));
        }
        instantiate(self.ctx, id, &args).map_err(|d| d.with_word(&self.name))
    }

    /// The innermost local of an enclosing word named `n`.
    fn capture(&self, n: &str) -> Option<Capture> {
        self.outer_locals
            .iter()
            .rev()
            .find(|c| c.name == n)
            .cloned()
    }

    /// Push the value of `n`, a local of an enclosing word, from the
    /// environment, recording it as captured. `None` if there is no such
    /// local; `E_CAPTURE` if it is mutable.
    fn read_capture(&mut self, n: &str, loc: &Location) -> Result<Option<Ty>, Diagnostic> {
        let Some(c) = self.capture(n) else {
            return Ok(None);
        };
        if c.mutable {
            return Err(self.capture_err(n, loc));
        }
        let k = match self.captures.iter().position(|x| x == n) {
            Some(k) => k,
            None => {
                debug_assert!(!self.emit, "a capture found only in the emitting pass");
                self.captures.push(n.to_string());
                self.captures.len() - 1
            }
        };
        if self.emit {
            let (t, local) = self.env_view.expect("an environment");
            let before: Vec<Ty> = self.captures[..k]
                .iter()
                .map(|x| self.capture(x).unwrap().ty)
                .collect();
            let field = 1 + self.lower_all(&before).len() as u32;
            for f in 0..self.lower(&c.ty).len() as u32 {
                self.op(I::LocalGet(local));
                self.op(I::StructGet {
                    struct_type_index: t,
                    field_index: field + f,
                });
            }
        }
        Ok(Some(c.ty))
    }

    fn capture_err(&self, n: &str, loc: &Location) -> Diagnostic {
        self.err(
            codes::E_CAPTURE,
            format!("a quotation value cannot capture `{n}`, a mutable local of the enclosing word; bind it immutably, or box shared state in a struct and capture that"),
            loc,
        )
    }

    /// Push the value of `n`, a local of this body or a captured one, for a
    /// quotation value created here that captures it.
    fn read_outer(&mut self, n: &str, loc: &Location) -> Result<(), Diagnostic> {
        if let Some(l) = self.locals.iter().rev().find(|l| l.name == n) {
            let idx = l.idx.clone();
            self.unstash(&idx);
            return Ok(());
        }
        self.read_capture(n, loc)?;
        Ok(())
    }

    fn name_ref(&mut self, n: &str, loc: &Location) -> Result<Flow, Diagnostic> {
        // Locals.
        if let Some(l) = self.locals.iter().find(|l| l.name == n) {
            let (ty, idx) = (l.ty.clone(), l.idx.clone());
            self.unstash(&idx);
            self.stack.push(ty);
            return Ok(Flow::Normal);
        }
        if let Some(ty) = self.read_capture(n, loc)? {
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
            if let Some(c) = self.capture(base) {
                if c.mutable {
                    return Err(self.capture_err(base, loc));
                }
                return Err(self.err(
                    codes::E_LOCAL,
                    format!("local `{base}` is immutable; bind it with `:> {base}!` to allow assignment"),
                    loc,
                ));
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
        if prims::is_raw(n) && !self.ctx.raw {
            return Err(self.err(
                codes::E_RAW,
                format!("`{n}` reaches memory by address: use it in a word marked `raw : ...`, or the checked words (`bytes`, `str`, arrays)"),
                loc,
            ));
        }
        // Numeric and memory instructions.
        if let Some((ins, outs, instr)) = prims::numeric(n) {
            self.pop_expect(n, &ins, loc)?;
            self.op(instr);
            self.stack.extend(outs);
            return Ok(Flow::Normal);
        }
        if n == "spawn" {
            let process = Ty::Quot(Box::new(Effect::new(Vec::new(), Vec::new())));
            self.pop_expect(n, &[process], loc)?;
            if self.emit {
                // The closure goes to the host through the `wack.spawn`
                // global (global 0, or 1 after `wack.frames`); the ring entry
                // carries its slot.
                let c = self.ctx.closure_type();
                let local = self.new_local(ref_ty(c));
                self.op(I::LocalTee(local));
                self.op(I::GlobalSet(if self.ctx.unwind != Unwind::Off {
                    1
                } else {
                    0
                }));
                self.op(I::LocalGet(local));
                self.op(I::StructGet {
                    struct_type_index: c,
                    field_index: 0,
                });
                self.op(I::I32Const(0));
                self.op(I::I32Const(0));
                self.op(I::I32Const(layout::OP_SPAWN));
                self.op(I::Call(FN_RING));
                self.op(I::Drop);
                self.ctx.uses_spawn = true;
            }
            return Ok(Flow::Normal);
        }
        if let Some((ins, outs)) = prims::special(n) {
            self.pop_expect(n, &ins, loc)?;
            match n {
                "str.len" | "bytes.len" => {
                    let mut ta = TempAlloc::default();
                    let t = self.temp(&mut ta, ValType::I32);
                    self.op(I::LocalSet(t));
                    self.op(I::Drop);
                    self.op(I::LocalGet(t));
                }
                "str.addr" | "bytes.addr" => self.op(I::Drop),
                "str.from-raw" | "bytes.from-raw" => {}
                "mem.alloc" => self.op(I::Call(FN_ALLOC)),
                // ( a0 a1 a2 op -- result ): exactly `rt.ring`'s parameters.
                "ring.submit" => match self.code.last() {
                    // A channel operation that may park: its opcode moves
                    // into the site's call.
                    Some(&I::I32Const(op))
                        if self.transformed
                            && matches!(
                                op,
                                layout::OP_CHAN_SEND
                                    | layout::OP_CHAN_RECV
                                    | layout::OP_ALT
                                    | layout::OP_SLEEP
                            ) =>
                    {
                        self.code.pop();
                        let ring = vec![I::I32Const(op), I::Call(FN_RING)];
                        let i3 = vec![ValType::I32; 3];
                        self.site_call(i3.clone(), vec![ValType::I32], ring.clone(), ring, i3);
                    }
                    _ => self.op(I::Call(FN_RING)),
                },
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
            "eq" | "hash" => {
                let op = if n == "eq" { Op::Eq } else { Op::Hash };
                let v = self.subst.fresh();
                let ins = if op == Op::Eq {
                    vec![v.clone(), v.clone()]
                } else {
                    vec![v.clone()]
                };
                self.pop_expect(n, &ins, loc)?;
                let t = self.subst.resolve(&v);
                if self.emit {
                    if t.has_var() {
                        return Err(self.err(
                            codes::E_AMBIGUOUS_TYPE,
                            format!(
                                "the type `{n}` works on is not known here; add a stack assertion"
                            ),
                            loc,
                        ));
                    }
                    self.hash_eq(op, &t, loc)?;
                }
                self.stack.push(Ty::I32);
                return Ok(Flow::Normal);
            }
            "array.new" => {
                self.pop_expect(n, &[Ty::I32], loc)?;
                let v = self.subst.fresh();
                let t = self.concrete_elem(&v, n, loc)?;
                let mut ta = TempAlloc::default();
                let c = self.temp(&mut ta, ValType::I32);
                self.op(I::LocalTee(c));
                if let Some(ti) = self.gc_array(&t) {
                    // Elements start as null references; reading a field of one traps.
                    self.op(I::ArrayNewDefault(ti));
                    self.op(I::I32Const(0));
                } else {
                    self.op(I::I32Const(t.elem_size() as i32));
                    self.op(I::I32Mul);
                    self.op(I::Call(FN_ALLOC));
                }
                self.op(I::LocalGet(c));
                self.stack.push(Ty::Array(Box::new(v)));
                return Ok(Flow::Normal);
            }
            "array.len" => {
                let e = self.pop_array(n, loc)?;
                let mut ta = TempAlloc::default();
                let t = self.temp(&mut ta, ValType::I32);
                self.op(I::LocalSet(t));
                self.op(I::Drop);
                if self.gc_array(&e).is_some() {
                    self.op(I::Drop);
                }
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
                let gc = self.gc_array(&t).map(|ti| {
                    let r = self.temp(&mut ta, ref_ty(ti));
                    self.op(I::LocalSet(r));
                    (ti, r)
                });
                self.op(I::LocalGet(idx));
                self.op(I::LocalGet(len));
                self.op(I::I32GeU);
                self.op(I::If(BlockType::Empty));
                self.trap(&format!("{n}: index out of bounds"));
                self.op(I::End);
                if let Some((ti, r)) = gc {
                    // `addr` holds the view's start.
                    self.op(I::LocalGet(r));
                    self.op(I::LocalGet(addr));
                    self.op(I::LocalGet(idx));
                    self.op(I::I32Add);
                    if store {
                        self.op(I::LocalGet(val[0]));
                        self.op(I::ArraySet(ti));
                    } else {
                        self.op(I::ArrayGet(ti));
                        self.stack.push(t);
                    }
                } else if store {
                    self.store_elem(&t, &ArrLocals::Linear(addr), idx, &val, &mut ta);
                } else {
                    self.load_elem(&t, &ArrLocals::Linear(addr), idx, &mut ta);
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
                let gc = self.gc_array(&t).is_some();
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
                // A struct-array view keeps its GC array (still on the stack
                // below) and moves its start; a linear view moves its address.
                self.op(I::LocalGet(addr));
                self.op(I::LocalGet(start));
                if !gc {
                    self.op(I::I32Const(t.elem_size() as i32));
                    self.op(I::I32Mul);
                }
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
                    // inputs.. closure -> inputs.. closure slot
                    let ti = self.ctx.func_type(&e, true);
                    let c = self.ctx.closure_type();
                    let local = self.new_local(ref_ty(c));
                    let rewound = vec![
                        I::LocalGet(local),
                        I::StructGet {
                            struct_type_index: c,
                            field_index: 0,
                        },
                        I::CallIndirect {
                            type_index: ti,
                            table_index: 0,
                        },
                    ];
                    let mut call = vec![I::LocalTee(local)];
                    call.extend(rewound.iter().cloned());
                    // Rewound, the restored closure is the call's last argument.
                    let mut rewound = rewound;
                    rewound.insert(0, I::LocalGet(local));
                    let args = e.wasm_params(&self.ctx.struct_types);
                    let mut ins = args.clone();
                    ins.push(ref_ty(c));
                    let outs = e.wasm_results(&self.ctx.struct_types);
                    self.site_call(ins, outs, call, rewound, args);
                }
                self.stack.extend(e.outputs.iter().cloned());
                return Ok(Flow::Normal);
            }
            _ => {}
        }
        // User and library words.
        if let Some(id) = self.ctx.lookup(n) {
            let (e, id) = if self.ctx.words[id].generic.is_some() {
                let (e, vars) = self.fresh_instance(id);
                self.pop_expect(n, &e.inputs, loc)?;
                if !self.emit {
                    self.stack.extend(e.outputs);
                    return Ok(Flow::Normal);
                }
                let inst = self.instance(id, n, &vars, loc)?;
                // The instance's concrete effect: its wasm type is the call's.
                (self.ctx.words[inst].effect.clone(), inst)
            } else {
                let e = self.ctx.words[id].effect.clone();
                self.pop_expect(n, &e.inputs, loc)?;
                (e, id)
            };
            if !self.ctx.indirect_calls || self.emit {
                self.word_call(id, &e);
            } else {
                self.callees.push((id, EdgeKind::Call));
            }
            self.stack.extend(e.outputs);
            return Ok(Flow::Normal);
        }
        Err(self.undefined(n, loc))
    }
}

/// Whether compiled code submits a ring entry whose opcode passes `op`: every
/// submission writes its opcode as a constant directly before `call rt.ring`.
pub fn submits(c: &Compiled, op: impl Fn(i32) -> bool) -> bool {
    c.code
        .windows(2)
        .any(|w| matches!(w, [I::I32Const(o), I::Call(f)] if *f == FN_RING && op(*o)))
}

/// Whether compiled code starts a process or uses a channel.
pub fn process_ops(c: &Compiled) -> bool {
    submits(c, |op| op >= layout::OP_SPAWN)
}

/// The placeholder body of a generic template: it traps, and is never
/// called (callers reach instances).
pub fn template_code(ctx: &mut Ctx, name: &str) -> Vec<I<'static>> {
    let (ma, ml) = ctx.intern_str(&format!("generic word `{name}` has no code of its own"));
    let (wa, wl) = ctx.intern_str(name);
    vec![
        I::I32Const(ma),
        I::I32Const(ml),
        I::I32Const(wa),
        I::I32Const(wl),
        I::Call(FN_TRAP),
        I::Unreachable,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_base_moves() {
        let mut ctx = Ctx::default();
        assert_eq!(ctx.intern_str("a"), (layout::LITERALS_BASE as i32, 1));
        ctx.begin_literals(0x30_0000);
        assert_eq!(ctx.intern_str("a"), (0x30_0000, 1));
        assert_eq!(ctx.literals, b"a");
        assert_eq!(ctx.intern_str("a"), (0x30_0000, 1));
    }

    fn call_code(indirect: bool) -> Vec<I<'static>> {
        let mut ctx = Ctx {
            indirect_calls: indirect,
            ..Ctx::default()
        };
        let e = Effect::new(vec![Ty::I32], vec![Ty::I32]);
        ctx.add_word(Word {
            name: "f".into(),
            effect: e.clone(),
            body: Some(Compiled {
                locals: vec![],
                code: vec![],
            }),
            failed: false,
            export: false,
            raw: false,
            origin: Origin::User,
            kind: WordKind::Named,
            loc: Location::default(),
            callees: vec![],
            inferred: false,
            generic: None,
            instance_of: None,
            generated: None,
        });
        let body = vec![Node {
            kind: NodeKind::Name("f".into()),
            loc: Location::default(),
        }];
        let out = compile_body(
            &mut ctx,
            "g",
            Mode::Declared(&e),
            &body,
            &Location::default(),
            &[],
        )
        .unwrap();
        out.compiled.code
    }

    /// A REPL line under `Unwind::All` (a step module: table calls, the
    /// line prologue and epilogue) mixing literals, binds, a site, a
    /// `leave` after a site in a `times` body and a `trap` in a run.
    #[test]
    fn a_transformed_line_validates() {
        let mut ctx = Ctx {
            indirect_calls: true,
            ..Ctx::default()
        };
        ctx.unwind = Unwind::All;
        let e = Effect::new(vec![Ty::I32], vec![Ty::I32]);
        let word = |name: &str, effect: Effect, kind, compiled| Word {
            name: name.into(),
            effect,
            body: Some(compiled),
            failed: false,
            export: false,
            raw: false,
            origin: Origin::User,
            kind,
            loc: Location::default(),
            callees: vec![],
            inferred: false,
            generic: None,
            instance_of: None,
            generated: None,
        };
        let f = ctx.add_word(word(
            "f",
            e.clone(),
            WordKind::Named,
            Compiled {
                locals: vec![],
                code: vec![I::LocalGet(0)],
            },
        ));
        let src = "1 :> x  x f :> y!  3 [ :> i  x f y i32.add y!  i 1 i32.eq [ leave ] when ] times                     y 100 i32.gt_s [ \"big\" trap ] when  y";
        let toks = crate::lexer::lex("line", src).unwrap();
        let crate::parser::ReplInput::Body(body) =
            crate::parser::parse_repl_with("line", &toks, &HashMap::new()).unwrap()
        else {
            panic!("a body");
        };
        let out = compile_body(
            &mut ctx,
            "[line 1]",
            Mode::Line(&[Ty::I32]),
            &body,
            &Location::default(),
            &[],
        )
        .unwrap();
        assert_eq!(out.effect.outputs, vec![Ty::I32, Ty::I32]);
        let skips = out
            .compiled
            .code
            .windows(2)
            .filter(|w| matches!(w, [I::I32Const(layout::REWINDING), I::I32Ne]))
            .count();
        assert!(skips >= 4, "the prologue and the runs are skipped: {skips}");
        let line = ctx.add_word(word("[line 1]", out.effect, WordKind::Line, out.compiled));
        let wasm = crate::module::assemble_step(&mut ctx, &[f, line], false);
        crate::validate(&wasm).unwrap();
    }

    /// A step module holding one `( ref -- ref )` word over a registered struct.
    fn struct_step(fields: impl Fn(u32) -> Vec<ValType>) -> (Ctx, Vec<u8>) {
        let mut ctx = Ctx::default();
        let base = ctx.types.len() as u32;
        let idx = ctx.register_struct_type(fields(base));
        assert_eq!(idx, base);
        assert_eq!(ctx.types.len() as u32, idx + 2);
        ctx.struct_types.insert("p".into(), (idx, idx + 1));
        let p = Ty::Struct("p".into(), Vec::new());
        let id = ctx.add_word(Word {
            name: "id".into(),
            effect: Effect::new(vec![p.clone()], vec![p]),
            body: Some(Compiled {
                locals: vec![],
                code: vec![I::LocalGet(0)],
            }),
            failed: false,
            export: false,
            raw: false,
            origin: Origin::User,
            kind: WordKind::Named,
            loc: Location::default(),
            callees: vec![],
            inferred: false,
            generic: None,
            instance_of: None,
            generated: None,
        });
        let bytes = crate::module::assemble_step(&mut ctx, &[id], false);
        (ctx, bytes)
    }

    #[test]
    fn check_types_allows_self() {
        let ctx = Ctx::default();
        let node = Ty::Struct("node".into(), Vec::new());
        let tys = [node.clone(), Ty::Array(Box::new(node))];
        let loc = Location::default();
        assert!(ctx.check_types(&tys, &loc, Some("node")).is_ok());
        assert_eq!(
            ctx.check_types(&tys, &loc, None).unwrap_err().code,
            codes::E_UNKNOWN_TYPE
        );
    }

    #[test]
    fn struct_types_validate() {
        use crate::types::ref_ty;
        let (_, b) = struct_step(|_| vec![ValType::I32, ValType::F64]);
        crate::program::validate(&b).unwrap();
        let (_, b) = struct_step(|i| vec![ref_ty(i), ValType::I32]);
        crate::program::validate(&b).unwrap();
        let (_, b) = struct_step(|i| vec![ref_ty(i + 1), ValType::I32, ValType::I32]);
        crate::program::validate(&b).unwrap();
    }

    fn infer(src: &str, n: usize) -> Result<Effect, Diagnostic> {
        let toks = crate::lexer::lex("t", &format!(": t ( -- ) {src} ;")).unwrap();
        let items = crate::parser::parse("t", &toks).unwrap();
        let crate::ast::Item::Def { body, .. } = &items[0] else {
            panic!()
        };
        let mut ctx = Ctx::default();
        let e = check_body(
            &mut ctx,
            "t",
            Mode::Infer(n),
            body,
            &Location::default(),
            &[],
        );
        assert!(ctx.words.is_empty());
        e
    }

    #[test]
    fn infer_mode() {
        assert_eq!(infer("1 i32.add", 1).unwrap().to_string(), "( i32 -- i32 )");
        assert_eq!(
            infer("1 i32.add", 0).unwrap_err().code,
            codes::E_STACK_UNDERFLOW
        );
        let e = infer("dup", 1).unwrap();
        assert!(matches!(e.inputs[..], [Ty::Var(_)]));
        assert_eq!(e.outputs, [e.inputs[0].clone(), e.inputs[0].clone()]);
    }

    fn line(src: &str, inputs: &[Ty]) -> Result<Output, Diagnostic> {
        let toks = crate::lexer::lex("t", &format!(": t ( -- ) {src} ;")).unwrap();
        let items = crate::parser::parse("t", &toks).unwrap();
        let crate::ast::Item::Def { body, .. } = &items[0] else {
            panic!()
        };
        let mut ctx = Ctx::default();
        compile_body(
            &mut ctx,
            "[line 1]",
            Mode::Line(inputs),
            body,
            &Location::default(),
            &[],
        )
    }

    #[test]
    fn line_refs_table() {
        let line_in = |src: &str, inputs: &[Ty]| {
            let toks = crate::lexer::lex("t", &format!(": t ( -- ) {src} ;")).unwrap();
            let items = crate::parser::parse("t", &toks).unwrap();
            let crate::ast::Item::Def { body, .. } = &items[0] else {
                panic!()
            };
            let mut ctx = Ctx::default();
            let idx = ctx.register_struct_type(vec![ValType::I32]);
            ctx.struct_types.insert("p".into(), (idx, idx + 1));
            ctx.struct_by_name.insert("p".into(), 0);
            compile_body(
                &mut ctx,
                "[line 1]",
                Mode::Line(inputs),
                body,
                &Location::default(),
                &[],
            )
            .unwrap()
            .compiled
            .code
        };
        let p = Ty::Struct("p".into(), Vec::new());
        let code = line_in("", std::slice::from_ref(&p));
        assert!(code.iter().any(|i| matches!(i, I::TableGet(1))));
        let code = line_in("0 array.new ( array p )", &[]);
        assert_eq!(
            code.iter().filter(|i| matches!(i, I::TableSet(1))).count(),
            1
        );
    }

    #[test]
    fn line_mode() {
        let out = line("1 i32.add", &[Ty::I32]).unwrap();
        assert_eq!(out.effect, Effect::new(vec![Ty::I32], vec![Ty::I32]));
        let code = &out.compiled.code;
        assert!(code
            .iter()
            .any(|i| matches!(i, I::I32Load(m) if m.offset == layout::DATA_STACK_PTR as u64)));
        assert!(code
            .iter()
            .any(|i| matches!(i, I::I32Store(m) if m.offset == 0)));
        assert_eq!(line("\"hi\"", &[]).unwrap().effect.outputs, vec![Ty::Str]);
        assert_eq!(
            line("5 array.new", &[]).err().unwrap().code,
            codes::E_AMBIGUOUS_TYPE
        );
        assert_eq!(
            line("i32.add", &[Ty::I32]).err().unwrap().code,
            codes::E_STACK_UNDERFLOW
        );
    }

    #[test]
    fn indirect_calls_use_the_table() {
        let code = call_code(true);
        assert!(code.iter().any(|i| matches!(i, I::CallIndirect { .. })));
        assert!(!code.iter().any(|i| matches!(i, I::Call(_))));
        let code = call_code(false);
        assert!(!code.iter().any(|i| matches!(i, I::CallIndirect { .. })));
        assert!(code.iter().any(|i| matches!(i, I::Call(_))));
    }
}
