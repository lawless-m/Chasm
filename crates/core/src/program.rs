//! Whole-program driver: sources in, diagnostics + word database + module out.

use serde::Serialize;

use crate::ast::{Item, Lit};
use crate::check::{compile_body, Ctx, Mode, Origin, Word, WordId, WordKind};
use crate::diag::{codes, Diagnostic, Location};
use crate::graph::{Edge, Graph};
use crate::lexer::lex;
use crate::module::{assemble, ModuleOptions};
use crate::parser::parse;
use crate::prims;
use crate::types::{names, Effect, Ty};

pub const PRELUDE: &str = include_str!("prelude.chasm");
pub const PRELUDE_NAME: &str = "<prelude>";

#[derive(Debug, Clone)]
pub struct Source {
    pub name: String,
    pub text: String,
}

impl Source {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        Source {
            name: name.into(),
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub prelude: bool,
    /// Export test thunks as `__test_N`.
    pub test_exports: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            prelude: true,
            test_exports: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum Value {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Str(String),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::I32(v) => write!(f, "{v}"),
            Value::I64(v) => write!(f, "{v}i64"),
            Value::F32(v) => write!(f, "{v:?}f32"),
            Value::F64(v) => write!(f, "{v:?}"),
            Value::Str(s) => write!(f, "{s:?}"),
        }
    }
}

impl From<&Lit> for Value {
    fn from(l: &Lit) -> Self {
        match l {
            Lit::I32(v) => Value::I32(*v),
            Lit::I64(v) => Value::I64(*v),
            Lit::F32(v) => Value::F32(*v),
            Lit::F64(v) => Value::F64(*v),
            Lit::Str(s) => Value::Str(s.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WordInfo {
    pub name: String,
    pub effect: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub resolved: bool,
    /// The body failed to check; an error was reported.
    pub failed: bool,
    pub export: bool,
    pub library: bool,
    pub location: Location,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestInfo {
    pub index: usize,
    pub word: String,
    /// Export name of the thunk in a module built with `test_exports`.
    pub export_name: String,
    pub expected: Vec<Value>,
    pub types: Vec<String>,
    pub pending: bool,
    pub location: Location,
    #[serde(skip)]
    pub result_types: Vec<Ty>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Compilation {
    pub diagnostics: Vec<Diagnostic>,
    #[serde(skip)]
    pub wasm: Option<Vec<u8>>,
    pub words: Vec<WordInfo>,
    pub tests: Vec<TestInfo>,
    pub graph: Graph,
    pub has_main: bool,
}

impl Compilation {
    pub fn ok(&self) -> bool {
        !self.diagnostics.iter().any(Diagnostic::is_error)
    }

    pub fn word(&self, name: &str) -> Option<&WordInfo> {
        self.words.iter().find(|w| w.name == name)
    }

    /// Declared user words with no body yet.
    pub fn unresolved(&self) -> Vec<&WordInfo> {
        self.words
            .iter()
            .filter(|w| !w.resolved && !w.failed && !w.library)
            .collect()
    }
}

pub fn compile(sources: &[Source], opts: &Options) -> Compilation {
    let mut ctx = Ctx::default();
    let mut diags = Vec::new();
    let mut tests: Vec<TestInfo> = Vec::new();
    let mut test_words: Vec<WordId> = Vec::new();

    let mut all: Vec<(Source, Origin)> = Vec::new();
    if opts.prelude {
        all.push((Source::new(PRELUDE_NAME, PRELUDE), Origin::Library));
    }
    all.extend(sources.iter().cloned().map(|s| (s, Origin::User)));

    // Parse everything first so "used before defined" can be explained.
    let mut parsed = Vec::new();
    for (src, origin) in &all {
        let items = lex(&src.name, &src.text).and_then(|t| parse(&src.name, &t));
        match items {
            Ok(items) => {
                for it in &items {
                    match it {
                        Item::Def { name, .. } | Item::Declare { name, .. } => {
                            ctx.all_names.insert(name.clone());
                        }
                        Item::Test { .. } => {}
                    }
                }
                parsed.push((items, *origin));
            }
            Err(d) => diags.push(d),
        }
    }

    for (items, origin) in parsed {
        for item in items {
            match item {
                Item::Declare { name, effect, loc } => {
                    if let Some(d) = check_new_name(&name, &loc) {
                        diags.push(d);
                        continue;
                    }
                    match ctx.by_name.get(&name) {
                        Some(&id) => {
                            if ctx.words[id].effect != effect {
                                diags.push(
                                    Diagnostic::error(
                                        codes::E_DECLARE_MISMATCH,
                                        format!(
                                            "`{name}` is already declared {} and cannot be declared {effect}",
                                            ctx.words[id].effect
                                        ),
                                        loc,
                                    )
                                    .with_word(&name),
                                );
                            }
                        }
                        None => {
                            ctx.add_word(new_word(name, effect, origin, WordKind::Named, loc));
                        }
                    }
                }
                Item::Def {
                    name,
                    effect,
                    body,
                    export,
                    loc,
                } => {
                    if let Some(d) = check_new_name(&name, &loc) {
                        diags.push(d);
                        continue;
                    }
                    let Some(effect) = effect else {
                        diags.push(
                            Diagnostic::error(
                                codes::E_SYNTAX,
                                format!("`: {name}` must be followed by its effect, e.g. `: {name} ( i32 -- i32 )`"),
                                loc,
                            )
                            .with_word(&name),
                        );
                        continue;
                    };
                    let id = match ctx.by_name.get(&name) {
                        Some(&id) => {
                            let w = &ctx.words[id];
                            if w.effect != effect {
                                let dependants = graph_of(&ctx).callers(&name);
                                let (code, what) = if w.body.is_none() && !w.failed {
                                    (codes::E_DECLARE_MISMATCH, "declared")
                                } else {
                                    (codes::E_REDEFINE_EFFECT, "defined")
                                };
                                let mut d = Diagnostic::error(
                                    code,
                                    format!(
                                        "`{name}` is {what} {} but this definition has effect {effect}; an effect can only change deliberately",
                                        w.effect
                                    ),
                                    loc,
                                )
                                .with_word(&name);
                                d.declared_effect = Some(w.effect.to_string());
                                d.dependants = Some(dependants);
                                diags.push(d);
                                continue;
                            }
                            id
                        }
                        None => ctx.add_word(new_word(
                            name.clone(),
                            effect.clone(),
                            origin,
                            WordKind::Named,
                            loc.clone(),
                        )),
                    };
                    if export {
                        ctx.words[id].export = true;
                    }
                    match compile_body(&mut ctx, &name, Mode::Declared(&effect), &body, &loc, &[]) {
                        Ok(out) => {
                            let w = &mut ctx.words[id];
                            w.body = Some(out.compiled);
                            w.callees = out.callees;
                            w.failed = false;
                            w.loc = loc;
                        }
                        Err(d) => {
                            ctx.words[id].failed = true;
                            diags.push(d);
                        }
                    }
                }
                Item::Test {
                    word,
                    body,
                    expected,
                    loc,
                } => {
                    if !ctx.by_name.contains_key(&word) && !prims::is_builtin(&word) {
                        let msg = if ctx.all_names.contains(&word) {
                            format!("test of `{word}` comes before `{word}` is defined; move the test below it or `declare` the word first")
                        } else {
                            format!("test names unknown word `{word}`")
                        };
                        diags.push(Diagnostic::error(codes::E_UNDEFINED, msg, loc));
                        continue;
                    }
                    let index = tests.len();
                    let tname = format!("[test {word} #{index}]");
                    let out = match compile_body(&mut ctx, &tname, Mode::Derived, &body, &loc, &[])
                    {
                        Ok(o) => o,
                        Err(d) => {
                            diags.push(d);
                            continue;
                        }
                    };
                    let want: Vec<Ty> = expected.iter().map(|(l, _)| l.ty()).collect();
                    if want != out.effect.outputs {
                        diags.push(
                            Diagnostic::error(
                                codes::E_TEST_TYPE,
                                format!(
                                    "test of `{word}` expects ( {} ) but its body leaves ( {} )",
                                    names(&want).join(" "),
                                    names(&out.effect.outputs).join(" ")
                                ),
                                loc,
                            )
                            .with_word(&word)
                            .with_stacks(names(&want), names(&out.effect.outputs)),
                        );
                        continue;
                    }
                    let tid = ctx.add_word(Word {
                        name: tname,
                        effect: out.effect.clone(),
                        body: Some(out.compiled),
                        failed: false,
                        export: false,
                        origin,
                        kind: WordKind::Test,
                        loc: loc.clone(),
                        callees: out.callees,
                    });
                    test_words.push(tid);
                    tests.push(TestInfo {
                        index,
                        word,
                        export_name: format!("__test_{index}"),
                        expected: expected.iter().map(|(l, _)| Value::from(l)).collect(),
                        types: names(&want),
                        pending: false,
                        location: loc,
                        result_types: want,
                    });
                }
            }
        }
    }

    // main must be ( -- ).
    let has_main = match ctx.by_name.get("main") {
        Some(&id) => {
            let w = &ctx.words[id];
            if !w.effect.inputs.is_empty() || !w.effect.outputs.is_empty() {
                diags.push(
                    Diagnostic::error(
                        codes::E_MAIN_EFFECT,
                        format!(
                            "`main` must have effect ( -- ) but is declared {}",
                            w.effect
                        ),
                        w.loc.clone(),
                    )
                    .with_word("main"),
                );
            }
            true
        }
        None => false,
    };

    for t in &mut tests {
        // Tests may also name primitives, which are never pending.
        t.pending = ctx
            .by_name
            .get(&t.word)
            .is_some_and(|&id| ctx.words[id].body.is_none());
    }

    let graph = graph_of(&ctx);
    let words = ctx
        .words
        .iter()
        .filter(|w| w.kind == WordKind::Named)
        .map(|w| WordInfo {
            name: w.name.clone(),
            effect: w.effect.to_string(),
            inputs: names(&w.effect.inputs),
            outputs: names(&w.effect.outputs),
            resolved: w.body.is_some(),
            failed: w.failed,
            export: w.export,
            library: w.origin == Origin::Library,
            location: w.loc.clone(),
        })
        .collect();

    let ok = !diags.iter().any(Diagnostic::is_error);
    let wasm = if ok {
        let test_exports = if opts.test_exports {
            tests
                .iter()
                .zip(&test_words)
                .map(|(t, &id)| (t.export_name.clone(), id))
                .collect()
        } else {
            Vec::new()
        };
        let bytes = assemble(&mut ctx, &ModuleOptions { test_exports });
        match validate(&bytes) {
            Ok(()) => Some(bytes),
            Err(e) => {
                diags.push(Diagnostic::error(
                    codes::E_INTERNAL,
                    format!("compiler produced an invalid module (this is a compiler bug): {e}"),
                    Location::default(),
                ));
                None
            }
        }
    } else {
        None
    };

    Compilation {
        diagnostics: diags,
        wasm,
        words,
        tests,
        graph,
        has_main,
    }
}

pub fn validate(bytes: &[u8]) -> Result<(), String> {
    wasmparser::Validator::new()
        .validate_all(bytes)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn new_word(name: String, effect: Effect, origin: Origin, kind: WordKind, loc: Location) -> Word {
    Word {
        name,
        effect,
        body: None,
        failed: false,
        export: false,
        origin,
        kind,
        loc,
        callees: Vec::new(),
    }
}

fn check_new_name(name: &str, loc: &Location) -> Option<Diagnostic> {
    if prims::is_builtin(name)
        || matches!(name, ":>" | "[" | "]" | "(" | ")" | ";" | ":" | "--" | "->")
    {
        return Some(Diagnostic::error(
            codes::E_REDEFINE_EFFECT,
            format!("`{name}` is a primitive and cannot be defined or declared"),
            loc.clone(),
        ));
    }
    if crate::parser::parse_number(name).is_some() || name.starts_with('\'') {
        return Some(Diagnostic::error(
            codes::E_SYNTAX,
            format!("`{name}` is not a valid word name"),
            loc.clone(),
        ));
    }
    None
}

fn graph_of(ctx: &Ctx) -> Graph {
    let mut g = Graph::default();
    for w in &ctx.words {
        if w.kind == WordKind::Test {
            continue;
        }
        let edges = w.callees.iter().map(|&(id, kind)| Edge {
            word: ctx.words[id].name.clone(),
            kind,
        });
        g.set_callees(&w.name, edges);
    }
    g
}
