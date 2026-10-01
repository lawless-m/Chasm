//! Checker types, effects and their wasm lowering.

use std::collections::HashMap;
use std::fmt;
use wasm_encoder::ValType;

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

impl Effect {
    pub fn new(inputs: Vec<Ty>, outputs: Vec<Ty>) -> Self {
        Effect {
            inputs,
            outputs,
            row: None,
        }
    }

    pub fn wasm_params(&self) -> Vec<ValType> {
        lower_all(&self.inputs)
    }

    pub fn wasm_results(&self) -> Vec<ValType> {
        lower_all(&self.outputs)
    }
}

impl Ty {
    /// Wasm value types, in stack order. Unresolved variables lower to `i32`
    /// (only ever seen during the checking pass, whose code is discarded).
    pub fn lower(&self) -> Vec<ValType> {
        match self {
            Ty::I32 | Ty::Quot(_) | Ty::Var(_) => vec![ValType::I32],
            Ty::I64 => vec![ValType::I64],
            Ty::F32 => vec![ValType::F32],
            Ty::F64 => vec![ValType::F64],
            Ty::Str | Ty::Array(_) => vec![ValType::I32, ValType::I32],
        }
    }

    /// Size in bytes as an array element (natural size; `str` is two `i32`s).
    pub fn elem_size(&self) -> u32 {
        match self {
            Ty::I32 | Ty::F32 | Ty::Quot(_) | Ty::Var(_) => 4,
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

pub fn lower_all(tys: &[Ty]) -> Vec<ValType> {
    tys.iter().flat_map(Ty::lower).collect()
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
    fn unify_array_var() {
        let mut s = Subst::default();
        let v = s.fresh();
        let a = Ty::Array(Box::new(v.clone()));
        assert!(s.unify(&a, &Ty::Array(Box::new(Ty::F64))));
        assert_eq!(s.resolve(&v), Ty::F64);
        assert!(!s.unify(&v, &Ty::I32));
    }
}
