//! Checker types, effects and their wasm lowering.

use std::collections::HashMap;
use std::fmt;
use wasm_encoder::{HeapType, RefType, ValType};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    I32,
    I64,
    F32,
    F64,
    Str,
    /// `bytes`: a mutable byte buffer, lowered like `str` to `i32 i32`
    /// (address, length).
    Bytes,
    /// `array T`: lowered to `i32 i32` (address, element count).
    Array(Box<Ty>),
    /// `[ effect ]`: a function value, a reference to a closure struct
    /// holding the function's table slot.
    Quot(Box<Effect>),
    /// A declared struct or union, by name, with its type arguments (empty
    /// unless the type is generic): a WasmGC reference.
    Struct(String, Vec<Ty>),
    /// A type variable of the checker's own, from polymorphic primitives
    /// (`array.new`, element types) and inference.
    Var(u32),
    /// A type variable written by the user, an uppercase-initial name such
    /// as `T`. Rigid: it unifies only with itself.
    Param(String),
}

/// A stack effect. `row` is reserved for a "rest of stack" row variable
/// (M6); v1 always leaves it `None`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Effect {
    pub inputs: Vec<Ty>,
    pub outputs: Vec<Ty>,
    pub row: Option<u32>,
}

/// A registered struct or union type, by its display name (`point`,
/// `pair i32 str`), to its wasm struct type index and the index of the GC
/// array type of its elements.
pub type StructTypes = HashMap<String, (u32, u32)>;

/// The `struct_types` key of the closure type, `$closure`: a non-final
/// struct holding a table slot, the supertype of every environment.
pub const CLOSURE: &str = "[closure]";

/// The `struct_types` key of the frame type, `$frame`: a non-final struct
/// (next frame, call-site index), the supertype of every call site's frame
/// in a transformed function (M12).
pub const FRAME: &str = "[frame]";

/// A nullable reference to concrete type `index` (nullable so locals are
/// defaultable).
pub fn ref_ty(index: u32) -> ValType {
    ValType::Ref(RefType {
        nullable: true,
        heap_type: HeapType::Concrete(index),
    })
}

impl Effect {
    pub fn new(inputs: Vec<Ty>, outputs: Vec<Ty>) -> Self {
        Effect {
            inputs,
            outputs,
            row: None,
        }
    }

    pub fn wasm_params(&self, structs: &StructTypes) -> Vec<ValType> {
        lower_all(&self.inputs, structs)
    }

    pub fn wasm_results(&self, structs: &StructTypes) -> Vec<ValType> {
        lower_all(&self.outputs, structs)
    }

    /// Whether the effect has type parameters, making its word generic.
    pub fn is_generic(&self) -> bool {
        self.inputs.iter().chain(&self.outputs).any(Ty::has_param)
    }

    /// The distinct type parameters, in order of first appearance.
    pub fn params(&self) -> Vec<String> {
        let mut out = Vec::new();
        for t in self.inputs.iter().chain(&self.outputs) {
            t.collect_params(&mut out);
        }
        out
    }

    pub fn substitute(&self, map: &HashMap<String, Ty>) -> Effect {
        Effect {
            inputs: self.inputs.iter().map(|t| t.substitute(map)).collect(),
            outputs: self.outputs.iter().map(|t| t.substitute(map)).collect(),
            row: self.row,
        }
    }
}

impl Ty {
    /// Wasm value types, in stack order. Unresolved variables lower to `i32`
    /// (only ever seen during the checking pass, whose code is discarded).
    /// A struct is one reference; an array of structs is a view over a
    /// WasmGC array: `( ref start len )`. A function value is a reference
    /// to `$closure` once that is registered, else an `i32` placeholder;
    /// an array of them is a view like an array of structs.
    ///
    /// An applied generic type that is not registered (only the checking
    /// pass meets one, and discards its code) lowers to an `i32` placeholder.
    pub fn lower(&self, structs: &StructTypes) -> Vec<ValType> {
        let index = |t: &Ty, array: bool| -> ValType {
            let name = t.to_string();
            match structs.get(&name) {
                Some(&(s, a)) => ref_ty(if array { a } else { s }),
                None if matches!(t, Ty::Struct(_, args) if !args.is_empty())
                    || t.has_var()
                    || t.has_param() =>
                {
                    ValType::I32
                }
                None => panic!("struct `{name}` lowered before it was declared"),
            }
        };
        match self {
            Ty::Quot(_) => match structs.get(CLOSURE) {
                Some(&(c, _)) if !self.has_var() && !self.has_param() => vec![ref_ty(c)],
                _ => vec![ValType::I32],
            },
            Ty::I32 | Ty::Var(_) | Ty::Param(_) => vec![ValType::I32],
            Ty::I64 => vec![ValType::I64],
            Ty::F32 => vec![ValType::F32],
            Ty::F64 => vec![ValType::F64],
            Ty::Struct(..) => vec![index(self, false)],
            Ty::Array(e) => match e.as_ref() {
                Ty::Struct(..) => vec![index(e, true), ValType::I32, ValType::I32],
                Ty::Quot(_) => {
                    let r = match structs.get(CLOSURE) {
                        Some(&(_, a)) if mentions_quot(e) => ref_ty(a),
                        _ => ValType::I32,
                    };
                    vec![r, ValType::I32, ValType::I32]
                }
                _ => vec![ValType::I32, ValType::I32],
            },
            Ty::Str | Ty::Bytes => vec![ValType::I32, ValType::I32],
        }
    }

    /// The number of wasm values the type occupies.
    pub fn width(&self) -> u32 {
        match self {
            Ty::Str | Ty::Bytes => 2,
            Ty::Array(e) if matches!(e.as_ref(), Ty::Struct(..) | Ty::Quot(_)) => 3,
            Ty::Array(_) => 2,
            _ => 1,
        }
    }

    /// Size in bytes as an array element (natural size; `str` and `bytes` are two `i32`s).
    /// Never used for structs: arrays of structs are GC arrays with no byte layout.
    pub fn elem_size(&self) -> u32 {
        match self {
            Ty::I32 | Ty::F32 | Ty::Quot(_) | Ty::Var(_) | Ty::Param(_) | Ty::Struct(..) => 4,
            Ty::I64 | Ty::F64 | Ty::Str | Ty::Bytes | Ty::Array(_) => 8,
        }
    }

    pub fn has_var(&self) -> bool {
        match self {
            Ty::Var(_) => true,
            Ty::Array(t) => t.has_var(),
            Ty::Struct(_, args) => args.iter().any(Ty::has_var),
            Ty::Quot(e) => e.inputs.iter().chain(&e.outputs).any(Ty::has_var),
            _ => false,
        }
    }

    pub fn has_param(&self) -> bool {
        match self {
            Ty::Param(_) => true,
            Ty::Array(t) => t.has_param(),
            Ty::Struct(_, args) => args.iter().any(Ty::has_param),
            Ty::Quot(e) => e.is_generic(),
            _ => false,
        }
    }

    /// Replace type parameters by name; unmapped ones stay.
    pub fn substitute(&self, map: &HashMap<String, Ty>) -> Ty {
        match self {
            Ty::Param(p) => map.get(p).cloned().unwrap_or_else(|| self.clone()),
            Ty::Array(t) => Ty::Array(Box::new(t.substitute(map))),
            Ty::Struct(n, args) => {
                Ty::Struct(n.clone(), args.iter().map(|a| a.substitute(map)).collect())
            }
            Ty::Quot(e) => Ty::Quot(Box::new(e.substitute(map))),
            _ => self.clone(),
        }
    }

    fn collect_params(&self, out: &mut Vec<String>) {
        match self {
            Ty::Param(p) if !out.contains(p) => out.push(p.clone()),
            Ty::Array(t) => t.collect_params(out),
            Ty::Struct(_, args) => {
                for a in args {
                    a.collect_params(out);
                }
            }
            Ty::Quot(e) => {
                for t in e.inputs.iter().chain(&e.outputs) {
                    t.collect_params(out);
                }
            }
            _ => {}
        }
    }
}

pub fn lower_all(tys: &[Ty], structs: &StructTypes) -> Vec<ValType> {
    tys.iter().flat_map(|t| t.lower(structs)).collect()
}

/// The struct and union types `t` mentions directly (inside arrays and
/// quotation effects, but not inside another struct's type arguments).
pub fn applied_types(t: &Ty, out: &mut Vec<Ty>) {
    match t {
        Ty::Struct(..) => {
            if !out.contains(t) {
                out.push(t.clone());
            }
        }
        Ty::Array(e) => applied_types(e, out),
        Ty::Quot(e) => {
            for t in e.inputs.iter().chain(&e.outputs) {
                applied_types(t, out);
            }
        }
        _ => {}
    }
}

/// Whether `t` mentions a concrete function value type, which needs
/// `$closure`.
pub fn mentions_quot(t: &Ty) -> bool {
    match t {
        Ty::Quot(_) => !t.has_var() && !t.has_param(),
        Ty::Array(e) => mentions_quot(e),
        Ty::Struct(_, args) => args.iter().any(mentions_quot),
        _ => false,
    }
}

pub fn width_all(tys: &[Ty]) -> u32 {
    tys.iter().map(Ty::width).sum()
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::I32 => write!(f, "i32"),
            Ty::I64 => write!(f, "i64"),
            Ty::F32 => write!(f, "f32"),
            Ty::F64 => write!(f, "f64"),
            Ty::Str => write!(f, "str"),
            Ty::Bytes => write!(f, "bytes"),
            Ty::Array(t) => write!(f, "array {t}"),
            Ty::Quot(e) => {
                write!(f, "[")?;
                for t in &e.inputs {
                    write!(f, " {t}")?;
                }
                write!(f, " --")?;
                for t in &e.outputs {
                    write!(f, " {t}")?;
                }
                write!(f, " ]")
            }
            Ty::Struct(name, args) => {
                write!(f, "{name}")?;
                for a in args {
                    write!(f, " {a}")?;
                }
                Ok(())
            }
            Ty::Var(n) => write!(f, "?{n}"),
            Ty::Param(p) => write!(f, "{p}"),
        }
    }
}

impl fmt::Display for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(")?;
        for t in &self.inputs {
            write!(f, " {t}")?;
        }
        write!(f, " --")?;
        for t in &self.outputs {
            write!(f, " {t}")?;
        }
        write!(f, " )")
    }
}

pub fn names(tys: &[Ty]) -> Vec<String> {
    tys.iter().map(|t| t.to_string()).collect()
}

/// Substitution for type variables, with unification.
#[derive(Debug, Clone, Default)]
pub struct Subst {
    map: HashMap<u32, Ty>,
    next: u32,
}

impl Subst {
    pub fn fresh(&mut self) -> Ty {
        let v = self.next;
        self.next += 1;
        Ty::Var(v)
    }

    /// Restart variable numbering, keeping bindings. The emitting pass
    /// re-walks a body with the checking pass's substitution.
    pub fn restart(&mut self) {
        self.next = 0;
    }

    pub fn resolve(&self, t: &Ty) -> Ty {
        match t {
            Ty::Var(v) => match self.map.get(v) {
                Some(t) => self.resolve(t),
                None => t.clone(),
            },
            Ty::Array(e) => Ty::Array(Box::new(self.resolve(e))),
            Ty::Struct(n, args) => Ty::Struct(n.clone(), self.resolve_all(args)),
            Ty::Quot(e) => Ty::Quot(Box::new(Effect {
                inputs: e.inputs.iter().map(|t| self.resolve(t)).collect(),
                outputs: e.outputs.iter().map(|t| self.resolve(t)).collect(),
                row: e.row,
            })),
            _ => t.clone(),
        }
    }

    pub fn resolve_all(&self, tys: &[Ty]) -> Vec<Ty> {
        tys.iter().map(|t| self.resolve(t)).collect()
    }

    fn occurs(&self, v: u32, t: &Ty) -> bool {
        match self.resolve(t) {
            Ty::Var(w) => v == w,
            Ty::Array(e) => self.occurs(v, &e),
            Ty::Struct(_, args) => args.iter().any(|t| self.occurs(v, t)),
            Ty::Quot(e) => e.inputs.iter().chain(&e.outputs).any(|t| self.occurs(v, t)),
            _ => false,
        }
    }

    pub fn unify(&mut self, a: &Ty, b: &Ty) -> bool {
        let a = self.resolve(a);
        let b = self.resolve(b);
        match (&a, &b) {
            (Ty::Var(x), Ty::Var(y)) if x == y => true,
            (Ty::Var(x), t) | (t, Ty::Var(x)) => {
                if self.occurs(*x, t) {
                    return false;
                }
                // Arrays of arrays are not v1.
                self.map.insert(*x, t.clone());
                true
            }
            (Ty::Array(x), Ty::Array(y)) => self.unify(x, y),
            (Ty::Struct(a, xs), Ty::Struct(b, ys)) => a == b && self.unify_all(xs, ys),
            (Ty::Quot(x), Ty::Quot(y)) => {
                x.inputs.len() == y.inputs.len()
                    && x.outputs.len() == y.outputs.len()
                    && x.inputs
                        .iter()
                        .zip(&y.inputs)
                        .chain(x.outputs.iter().zip(&y.outputs))
                        .all(|(p, q)| self.unify(p, q))
            }
            _ => a == b,
        }
    }

    /// Unify two type lists pairwise. Lengths must match.
    pub fn unify_all(&mut self, a: &[Ty], b: &[Ty]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| self.unify(x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display() {
        let e = Effect::new(
            vec![
                Ty::Array(Box::new(Ty::I32)),
                Ty::Quot(Box::new(Effect::new(vec![Ty::I32], vec![Ty::I32]))),
            ],
            vec![Ty::Str],
        );
        assert_eq!(e.to_string(), "( array i32 [ i32 -- i32 ] -- str )");
    }

    #[test]
    fn structs() {
        let p = || Ty::Struct("p".into(), Vec::new());
        let e = Effect::new(vec![p()], vec![Ty::Array(Box::new(p()))]);
        assert_eq!(e.to_string(), "( p -- array p )");
        let widths: Vec<u32> = [
            Ty::Str,
            Ty::Array(Box::new(Ty::I32)),
            Ty::Array(Box::new(p())),
            p(),
        ]
        .iter()
        .map(Ty::width)
        .collect();
        assert_eq!(widths, [2, 2, 3, 1]);
        let map: StructTypes = [("p".to_string(), (3, 4))].into();
        assert_eq!(p().lower(&map), vec![ref_ty(3)]);
        assert_eq!(
            Ty::Array(Box::new(p())).lower(&map),
            vec![ref_ty(4), ValType::I32, ValType::I32]
        );
        let mut s = Subst::default();
        assert!(!s.unify(&p(), &Ty::Struct("q".into(), Vec::new())));
        assert!(s.unify(&p(), &p()));
    }

    #[test]
    fn unify_array_var() {
        let mut s = Subst::default();
        let v = s.fresh();
        let a = Ty::Array(Box::new(v.clone()));
        assert!(s.unify(&a, &Ty::Array(Box::new(Ty::F64))));
        assert_eq!(s.resolve(&v), Ty::F64);
        assert!(!s.unify(&v, &Ty::I32));
    }

    #[test]
    fn params() {
        let p = |n: &str| Ty::Param(n.into());
        let e = Effect::new(vec![p("T")], vec![p("T"), p("T")]);
        assert_eq!(e.to_string(), "( T -- T T )");
        assert!(e.is_generic());
        let q = Effect::new(
            vec![
                Ty::Array(Box::new(p("T"))),
                Ty::Quot(Box::new(Effect::new(vec![p("T")], vec![p("U")]))),
            ],
            vec![p("U")],
        );
        assert_eq!(q.to_string(), "( array T [ T -- U ] -- U )");
        let r = Effect::new(vec![p("U"), p("T")], vec![Ty::Array(Box::new(p("T")))]);
        assert_eq!(r.params(), ["U", "T"]);
        let mut s = Subst::default();
        assert!(s.unify(&p("T"), &p("T")));
        assert!(!s.unify(&p("T"), &p("U")));
        assert!(!s.unify(&p("T"), &Ty::I32));
        let v = s.fresh();
        assert!(s.unify(&v, &p("T")));
        assert_eq!(s.resolve(&v), p("T"));
        let map: HashMap<String, Ty> = [("T".to_string(), Ty::I32)].into();
        assert_eq!(e.substitute(&map).to_string(), "( i32 -- i32 i32 )");
        assert!(!Effect::new(vec![Ty::I32], vec![]).is_generic());
    }
}
