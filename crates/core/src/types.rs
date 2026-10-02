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
    /// `array T`: lowered to `i32 i32` (address, element count).
    Array(Box<Ty>),
    /// `[ effect ]`: a function table index.
    Quot(Box<Effect>),
    /// A declared struct, by name: a WasmGC reference.
    Struct(String),
    /// A type variable. Only arises from polymorphic primitives
    /// (`array.new`, element types); never written in user effects.
    Var(u32),
}

/// A stack effect. `row` is reserved for a "rest of stack" row variable
/// (M6); v1 always leaves it `None`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Effect {
    pub inputs: Vec<Ty>,
    pub outputs: Vec<Ty>,
    pub row: Option<u32>,
}

/// Struct name to the index of its wasm struct type in the module's type
/// section; the GC array type of that struct is the next index.
pub type StructTypes = HashMap<String, u32>;

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
}

impl Ty {
    /// Wasm value types, in stack order. Unresolved variables lower to `i32`
    /// (only ever seen during the checking pass, whose code is discarded).
    /// A struct is one reference; an array of structs is a view over a
    /// WasmGC array: `( ref start len )`.
    pub fn lower(&self, structs: &StructTypes) -> Vec<ValType> {
        let index = |name: &str| -> u32 {
            *structs
                .get(name)
                .unwrap_or_else(|| panic!("struct `{name}` lowered before it was declared"))
        };
        match self {
            Ty::I32 | Ty::Quot(_) | Ty::Var(_) => vec![ValType::I32],
            Ty::I64 => vec![ValType::I64],
            Ty::F32 => vec![ValType::F32],
            Ty::F64 => vec![ValType::F64],
            Ty::Struct(name) => vec![ref_ty(index(name))],
            Ty::Array(e) => match e.as_ref() {
                Ty::Struct(name) => vec![ref_ty(index(name) + 1), ValType::I32, ValType::I32],
                _ => vec![ValType::I32, ValType::I32],
            },
            Ty::Str => vec![ValType::I32, ValType::I32],
        }
    }

    /// The number of wasm values the type occupies.
    pub fn width(&self) -> u32 {
        match self {
            Ty::Str => 2,
            Ty::Array(e) if matches!(e.as_ref(), Ty::Struct(_)) => 3,
            Ty::Array(_) => 2,
            _ => 1,
        }
    }

    /// Size in bytes as an array element (natural size; `str` is two `i32`s).
    /// Never used for structs: arrays of structs are GC arrays with no byte layout.
    pub fn elem_size(&self) -> u32 {
        match self {
            Ty::I32 | Ty::F32 | Ty::Quot(_) | Ty::Var(_) | Ty::Struct(_) => 4,
            Ty::I64 | Ty::F64 | Ty::Str | Ty::Array(_) => 8,
        }
    }

    pub fn has_var(&self) -> bool {
        match self {
            Ty::Var(_) => true,
            Ty::Array(t) => t.has_var(),
            Ty::Quot(e) => e.inputs.iter().chain(&e.outputs).any(Ty::has_var),
            _ => false,
        }
    }
}

pub fn lower_all(tys: &[Ty], structs: &StructTypes) -> Vec<ValType> {
    tys.iter().flat_map(|t| t.lower(structs)).collect()
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
            Ty::Struct(name) => write!(f, "{name}"),
            Ty::Var(n) => write!(f, "?{n}"),
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
        let p = || Ty::Struct("p".into());
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
        let map: StructTypes = [("p".to_string(), 3)].into();
        assert_eq!(p().lower(&map), vec![ref_ty(3)]);
        assert_eq!(
            Ty::Array(Box::new(p())).lower(&map),
            vec![ref_ty(4), ValType::I32, ValType::I32]
        );
        let mut s = Subst::default();
        assert!(!s.unify(&p(), &Ty::Struct("q".into())));
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
}
