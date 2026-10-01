//! Abstract syntax.

use crate::diag::Location;
use crate::types::{Effect, Ty};

#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Str(String),
}

impl Lit {
    pub fn ty(&self) -> Ty {
        match self {
            Lit::I32(_) => Ty::I32,
            Lit::I64(_) => Ty::I64,
            Lit::F32(_) => Ty::F32,
            Lit::F64(_) => Ty::F64,
            Lit::Str(_) => Ty::Str,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    pub loc: Location,
}

pub type Body = Vec<Node>;

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Lit(Lit),
    /// A name: word, primitive, local, or `local!` assignment. Resolved by the checker.
    Name(String),
    /// `:> name` or `:> name!`.
    Bind {
        name: String,
        mutable: bool,
    },
    /// `'name`: a word's address.
    Tick(String),
    /// A quotation not under a combinator: a function value.
    Quote(Body),
    /// Stack assertion `( types )`.
    Assert(Vec<Ty>),
    If(Body, Body),
    When(Body),
    Unless(Body),
    While(Body, Body),
    Until(Body, Body),
    Times(Body),
    Leave,
    Each(Body),
    Map(Body),
    Filter(Body),
    Fold(Body),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Def {
        name: String,
        effect: Option<Effect>,
        body: Body,
        export: bool,
        loc: Location,
    },
    Declare {
        name: String,
        effect: Effect,
        loc: Location,
    },
    Test {
        word: String,
        body: Body,
        expected: Vec<(Lit, Location)>,
        loc: Location,
    },
}
