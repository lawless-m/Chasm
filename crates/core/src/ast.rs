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
    /// A quotation not under a combinator: a function value, with its
    /// effect if written directly after `[`.
    Quote {
        body: Body,
        effect: Option<Effect>,
    },
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
    /// `value v1: [ ... ] v2: [ ... ] else: [ ... ] match`, arms in the order
    /// written.
    Match(Vec<Arm>),
    /// `a recv: [ ... ] b recv: [ ... ] alt`, arms in the order written.
    Alt(Vec<AltArm>),
}

/// An arm of `alt`: the node that pushes its channel, and its body, which
/// receives `option T`. `loc` is the `recv:` label's.
#[derive(Debug, Clone, PartialEq)]
pub struct AltArm {
    pub chan: Node,
    pub body: Body,
    pub loc: Location,
}

/// A labelled arm of `match`; the label of `else:` is `else`.
#[derive(Debug, Clone, PartialEq)]
pub struct Arm {
    pub label: String,
    pub body: Body,
    pub loc: Location,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Def {
        name: String,
        effect: Option<Effect>,
        body: Body,
        export: bool,
        /// `raw :`: the body may use the words that reach memory by address.
        raw: bool,
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
        /// `-> trap`: the body must trap; `expected` is empty.
        traps: bool,
        loc: Location,
    },
    /// `struct name  field: type ...`
    Struct {
        name: String,
        /// Type parameters, `struct pair T U ...`.
        params: Vec<String>,
        fields: Vec<(String, Ty, Location)>,
        loc: Location,
    },
    /// `union name  | variant  field: type ...  | variant ...`
    Union {
        name: String,
        params: Vec<String>,
        variants: Vec<Variant>,
        loc: Location,
    },
}

/// One variant of a union, with its fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<(String, Ty, Location)>,
    pub loc: Location,
}
