//! Chasm compiler core.
//!
//! Lexer, parser, effect checker, dependency graph and wasm emitter.
//! No I/O: this crate builds unchanged for native targets and `wasm32`.

// Diagnostics are the error type everywhere; their size is irrelevant on the error path.
#![allow(clippy::result_large_err)]

pub mod ast;
pub mod check;
pub mod diag;
pub mod graph;
pub mod infer;
pub mod layout;
pub mod lexer;
pub mod module;
pub mod parser;
pub mod prims;
pub mod program;
pub mod repl;
pub mod types;
pub mod wasi;

pub use check::StructDef;
pub use diag::{Diagnostic, Location, Severity};
pub use program::{compile, validate, Compilation, Options, Source, TestInfo, Value, WordInfo};
pub use repl::{Session, Step};
