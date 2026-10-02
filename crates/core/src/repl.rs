//! The REPL session: a persistent word database compiled one step at a time.
//!
//! Pure data in, data out. Each step returns the module holding the words it
//! compiled plus everything the host must do to install and run it; the
//! hosts (wasmtime natively, the Web Worker in the browser) own the shared
//! memory and table.

use crate::ast::Item;
use crate::check::{compile_body, Ctx, Mode, Origin, StructDef, Word, WordId, WordKind};
use crate::diag::{codes, Diagnostic, Location};
use crate::layout;
use crate::lexer::lex;
use crate::module::{assemble_step, export_name};
use crate::parser::{parse, parse_repl, ReplInput};
use crate::program::Value;
use crate::program::{
    process_item, register_names, struct_words, validate, Program, PRELUDE, PRELUDE_NAME,
};
use crate::types::{names, width_all, Ty};
use serde::Serialize;
use std::collections::HashSet;

/// Install a step module's export `export` in the shared table at `slot`.
#[derive(Debug, Clone)]
pub struct Install {
    pub export: String,
    /// Table slot, which is the word id.
    pub slot: u32,
}

/// The step's anonymous line word.
#[derive(Debug, Clone)]
pub struct Line {
    pub slot: u32,
    pub export: String,
    /// The memory data stack's types after the line runs.
    pub stack_after: Vec<Ty>,
}

/// A test for the host to run now: call the thunk in table slot `slot`
/// with no arguments; its results are the lowered `result_types`.
#[derive(Debug, Clone)]
pub struct TestRun {
    pub slot: u32,
    pub word: String,
    pub expected: Vec<Value>,
    pub result_types: Vec<Ty>,
    pub types: Vec<String>,
    pub location: Location,
}

/// A word the step defined or declared.
#[derive(Debug, Clone, PartialEq)]
pub struct Defined {
    pub name: String,
    pub effect: String,
    /// Declared without a body.
    pub declared: bool,
}

/// The result of one REPL step.
///
/// Host contract: write `literal_bytes` at `literal_addr` and set the heap
/// pointer to `(literal_addr + len + 7) & !7`, even when the step failed
/// (interned addresses must stay valid); grow the table to `table_size`;
/// instantiate `module` with the imports `chasm.ring_enter`, `chasm.memory`
/// and `chasm.table`; set each install's slot to its export. If `line` is
/// Some, snapshot the memory data stack, call the line, and on success set
/// `Session::stack` to `stack_after`; on a trap restore the snapshot and
/// leave `Session::stack` alone.
#[derive(Debug, Clone)]
pub struct Step {
    pub diagnostics: Vec<Diagnostic>,
    /// The step module; `None` when nothing was compiled.
    pub module: Option<Vec<u8>>,
    /// Every function in `module`, including the line word.
    pub installs: Vec<Install>,
    /// The host grows the table to at least this.
    pub table_size: u32,
    pub literal_addr: u32,
    pub literal_bytes: Vec<u8>,
    /// Some only when the step had no errors and was a bare body.
    pub line: Option<Line>,
    pub defined: Vec<Defined>,
    /// Tests to run now: those typed in this step whose word has a body,
    /// and every test of a word this step gave a body. Empty if the step
    /// has errors.
    pub tests: Vec<TestRun>,
    /// The host grows the `chasm.refs` anyref table to at least this many
    /// entries before running the line. A struct on the memory data stack is
    /// one slot holding its own slot index (counted from `DATA_STACK_BASE`)
    /// into that table; an `array <struct>` is three slots: that index for
    /// its GC array, then `start` and `len` as plain `i32`s.
    pub refs_size: u32,
    /// Words a `)forget` command removed.
    pub forgotten: Vec<String>,
}

impl Step {
    pub fn ok(&self) -> bool {
        !self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

pub struct Session {
    ctx: Ctx,
    /// Types on the memory data stack, bottom to top; always concrete.
    pub stack: Vec<Ty>,
    program: Program,
    shared_memory: bool,
    steps: u32,
    /// Indices of tests whose word was forgotten; they never run again.
    forgotten_tests: HashSet<usize>,
}

impl Session {
    pub fn has_structs(&self) -> bool {
        !self.ctx.structs.is_empty()
    }

    pub fn structs(&self) -> &[StructDef] {
        &self.ctx.structs
    }

    pub fn struct_fields(&self, name: &str) -> Option<&[(String, Ty)]> {
        let &k = self.ctx.struct_by_name.get(name)?;
        Some(&self.ctx.structs[k].fields)
    }

    /// The table slot of a named word (its id), e.g. to call `point.x`.
    pub fn word_slot(&self, name: &str) -> Option<u32> {
        self.ctx.by_name.get(name).map(|&id| id as u32)
    }

    /// Start a session. `shared_memory` is for the browser, whose memory is
    /// shared with the worker. Returns the step installing the prelude.
    pub fn new(prelude: bool, shared_memory: bool, heap_ptr: u32) -> (Session, Step) {
        let mut ctx = Ctx::default();
        ctx.indirect_calls = true;
        ctx.begin_literals(heap_ptr);
        let mut s = Session {
            ctx,
            stack: Vec::new(),
            program: Program::default(),
            shared_memory,
            steps: 0,
            forgotten_tests: HashSet::new(),
        };
        let mut built = Vec::new();
        if prelude {
            match lex(PRELUDE_NAME, PRELUDE).and_then(|t| parse(PRELUDE_NAME, &t)) {
                Ok(items) => {
                    register_names(&mut s.ctx, &items);
                    for item in items {
                        built.extend(process_item(
                            &mut s.ctx,
                            item,
                            Origin::Library,
                            &mut s.program,
                        ));
                    }
                }
                Err(d) => s.program.diagnostics.push(d),
            }
        }
        let step = s.finish(heap_ptr, 0, 0, built, Vec::new(), None);
        (s, step)
    }

    /// Compile one chunk of input. `heap_ptr` is where the host will place
    /// this step's literals.
    pub fn step(&mut self, text: &str, heap_ptr: u32) -> Step {
        self.ctx.begin_literals(heap_ptr);
        if let Some(command) = text.trim_start().strip_prefix(')') {
            return self.command(command, heap_ptr);
        }
        self.steps += 1;
        let file = format!("<repl:{}>", self.steps);
        let before = self.ctx.words.len();
        let tests_before = self.program.tests.len();
        let mut built = Vec::new();
        let mut defined = Vec::new();
        let mut line = None;
        match lex(&file, text).and_then(|t| parse_repl(&file, &t)) {
            Err(d) => self.program.diagnostics.push(d),
            Ok(ReplInput::Items(items)) => {
                register_names(&mut self.ctx, &items);
                for item in items {
                    let named = matches!(
                        item,
                        Item::Def { .. } | Item::Declare { .. } | Item::Struct { .. }
                    );
                    for id in process_item(&mut self.ctx, item, Origin::User, &mut self.program) {
                        built.push(id);
                        if named {
                            let w = &self.ctx.words[id];
                            defined.push(Defined {
                                name: w.name.clone(),
                                effect: w.effect.to_string(),
                                declared: w.body.is_none(),
                            });
                        }
                    }
                }
            }
            Ok(ReplInput::Body(body)) => {
                let name = format!("[line {}]", self.steps);
                let loc = body.first().map(|n| n.loc.clone()).unwrap_or(Location {
                    file: file.clone(),
                    line: 1,
                    column: 1,
                    token: String::new(),
                });
                match compile_body(
                    &mut self.ctx,
                    &name,
                    Mode::Line(&self.stack),
                    &body,
                    &loc,
                    &[],
                ) {
                    Ok(out) => {
                        let stack_after = out.effect.outputs.clone();
                        let id = self.ctx.add_word(Word {
                            name,
                            effect: out.effect,
                            body: Some(out.compiled),
                            failed: false,
                            export: false,
                            origin: Origin::User,
                            kind: WordKind::Line,
                            loc,
                            callees: out.callees,
                        });
                        line = Some(Line {
                            slot: id as u32,
                            export: export_name(id),
                            stack_after,
                        });
                    }
                    Err(d) => self.program.diagnostics.push(d),
                }
            }
        }
        self.finish(heap_ptr, before, tests_before, built, defined, line)
    }

    /// A REPL command: a line starting with `)`, which no Chasm line can.
    /// Commands are not part of the language, so a file never holds one.
    fn command(&mut self, text: &str, heap_ptr: u32) -> Step {
        let loc = Location {
            file: "<repl>".to_string(),
            line: 1,
            column: 1,
            token: format!("){}", text.trim()),
        };
        let mut forgotten = Vec::new();
        match text.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["forget", name] => match self.forget(name, &loc) {
                Ok(()) => forgotten.push(name.to_string()),
                Err(d) => self.program.diagnostics.push(d),
            },
            _ => self.program.diagnostics.push(Diagnostic::error(
                codes::E_SYNTAX,
                format!(
                    "unknown REPL command `){}`; the command is `)forget word`",
                    text.trim()
                ),
                loc,
            )),
        }
        let before = self.ctx.words.len();
        let tests_before = self.program.tests.len();
        let mut step = self.finish(heap_ptr, before, tests_before, Vec::new(), Vec::new(), None);
        step.forgotten = forgotten;
        step
    }

    /// Remove a user word and its tests, freeing the name. Refused while any
    /// word or another word's test uses it. Its table slot is never reused,
    /// so a function value of it still on the stack keeps calling its code.
    fn forget(&mut self, name: &str, loc: &Location) -> Result<(), Diagnostic> {
        let refuse =
            |msg: String| Diagnostic::error(codes::E_FORGET, msg, loc.clone()).with_word(name);
        let Some(&id) = self.ctx.by_name.get(name) else {
            if crate::prims::is_builtin(name) {
                return Err(refuse(format!(
                    "`{name}` is a primitive and cannot be forgotten"
                )));
            }
            return Err(Diagnostic::error(
                codes::E_UNDEFINED,
                format!(
                    "unknown word `{name}`{}",
                    crate::prims::suggest(name, self.ctx.by_name.keys().map(String::as_str))
                ),
                loc.clone(),
            ));
        };
        if self.ctx.words[id].origin == Origin::Library {
            return Err(refuse(format!(
                "`{name}` is a prelude word and cannot be forgotten"
            )));
        }
        if let Some(sd) = self.ctx.structs.iter().find(|sd| {
            struct_words(&sd.name, &sd.fields, 0)
                .iter()
                .any(|(w, _, _)| w == name)
        }) {
            return Err(refuse(format!(
                "`{name}` is generated by struct `{}` and cannot be forgotten on its own",
                sd.name
            )));
        }
        let dependants = self.dependants(id, name);
        if !dependants.is_empty() {
            let mut d = refuse(format!(
                "`{name}` is still used by {}; forget those first",
                dependants.join(", ")
            ));
            d.dependants = Some(dependants);
            return Err(d);
        }
        self.ctx.by_name.remove(name);
        self.ctx.all_names.remove(name);
        let w = &mut self.ctx.words[id];
        w.name = format!("[forgotten {name}]");
        w.callees.clear();
        for t in &self.program.tests {
            if t.word == name {
                self.forgotten_tests.insert(t.index);
            }
        }
        Ok(())
    }

    /// The named words, and the tests of other words, that use word `id`
    /// directly or through quotations. REPL lines do not count.
    fn dependants(&self, id: WordId, name: &str) -> Vec<String> {
        let test_of: std::collections::HashMap<WordId, &str> = self
            .program
            .tests
            .iter()
            .filter(|t| !self.forgotten_tests.contains(&t.index))
            .map(|t| (self.program.test_words[t.index], t.word.as_str()))
            .collect();
        let mut seen = HashSet::from([id]);
        let mut todo = vec![id];
        let mut out = Vec::new();
        while let Some(target) = todo.pop() {
            for (caller, w) in self.ctx.words.iter().enumerate() {
                if !w.callees.iter().any(|&(c, _)| c == target) || !seen.insert(caller) {
                    continue;
                }
                match w.kind {
                    WordKind::Named => out.push(w.name.clone()),
                    WordKind::Quote => todo.push(caller),
                    WordKind::Test => match test_of.get(&caller) {
                        Some(&word) if word != name => out.push(format!("test {word}")),
                        _ => {}
                    },
                    WordKind::Line => {}
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Build the step module over the ids added since `before` plus `built`.
    /// Every line takes a table slot that is never freed (acceptable in v1).
    fn finish(
        &mut self,
        heap_ptr: u32,
        before: WordId,
        tests_before: usize,
        built: Vec<WordId>,
        defined: Vec<Defined>,
        line: Option<Line>,
    ) -> Step {
        let tests = self.tests_to_run(tests_before, &built);
        let mut ids: Vec<WordId> = (before..self.ctx.words.len()).chain(built).collect();
        ids.sort();
        ids.dedup();
        let mut diagnostics = std::mem::take(&mut self.program.diagnostics);
        let mut module = None;
        let mut installs = Vec::new();
        if !ids.is_empty() {
            let bytes = assemble_step(&mut self.ctx, &ids, self.shared_memory);
            match validate(&bytes) {
                Ok(()) => {
                    module = Some(bytes);
                    installs = ids
                        .iter()
                        .map(|&id| Install {
                            export: export_name(id),
                            slot: id as u32,
                        })
                        .collect();
                }
                Err(e) => diagnostics.push(Diagnostic::error(
                    crate::diag::codes::E_INTERNAL,
                    format!("compiler produced an invalid module (this is a compiler bug): {e}"),
                    Location::default(),
                )),
            }
        }
        let ok = !diagnostics.iter().any(Diagnostic::is_error);
        let refs_size = match &line {
            Some(l) if self.has_structs() => width_all(&self.stack).max(width_all(&l.stack_after)),
            _ => 0,
        };
        Step {
            diagnostics,
            module,
            installs,
            table_size: self.ctx.words.len() as u32,
            literal_addr: heap_ptr,
            literal_bytes: self.ctx.literals.clone(),
            line: if ok { line } else { None },
            defined,
            tests: if ok { tests } else { Vec::new() },
            refs_size,
            forgotten: Vec::new(),
        }
    }

    fn tests_to_run(&self, tests_before: usize, built: &[WordId]) -> Vec<TestRun> {
        let has_body = |word: &str| match self.ctx.by_name.get(word) {
            Some(&id) => self.ctx.words[id].body.is_some(),
            None => true, // a primitive
        };
        let given_body: Vec<&str> = built
            .iter()
            .filter(|&&id| self.ctx.words[id].body.is_some())
            .map(|&id| self.ctx.words[id].name.as_str())
            .collect();
        self.program
            .tests
            .iter()
            .filter(|t| !self.forgotten_tests.contains(&t.index))
            .filter(|t| {
                (t.index >= tests_before && has_body(&t.word))
                    || given_body.contains(&t.word.as_str())
            })
            .map(|t| TestRun {
                slot: self.program.test_words[t.index] as u32,
                word: t.word.clone(),
                expected: t.expected.clone(),
                result_types: t.result_types.clone(),
                types: names(&t.result_types),
                location: t.location.clone(),
            })
            .collect()
    }
}

/// Whether a chunk is incomplete and the REPL should read another line:
/// a `:` definition or a `[` quotation is still open. A lex error is left
/// for the step to report.
pub fn needs_more(text: &str) -> bool {
    let Ok(toks) = lex("<repl>", text) else {
        return false;
    };
    if toks
        .first()
        .is_some_and(|t| t.is("declare") || t.is("test") || t.is("struct"))
    {
        return false;
    }
    let mut in_def = false;
    let mut depth = 0u32;
    for t in &toks {
        if t.is(":") || t.is("export") {
            in_def = true;
        } else if t.is(";") {
            in_def = false;
        } else if t.is("[") {
            depth += 1;
        } else if t.is("]") {
            depth = depth.saturating_sub(1);
        }
    }
    in_def || depth > 0
}

/// One value on the memory data stack, for display.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StackEntry {
    pub ty: String,
    pub value: String,
}

/// Bytes of the memory data stack that `types` occupy.
pub fn stack_bytes(types: &[Ty]) -> u32 {
    width_all(types) * layout::STACK_SLOT
}

/// Render the memory data stack, bottom to top. Stops early if `mem` is
/// too short.
pub fn read_stack(
    mem: &[u8],
    types: &[Ty],
    refs: &mut dyn FnMut(u32, &str) -> Value,
) -> Vec<StackEntry> {
    let slot = |i: u32| -> Option<[u8; 8]> {
        let a = (layout::DATA_STACK_BASE + i * layout::STACK_SLOT) as usize;
        mem.get(a..a + 8).map(|b| b.try_into().unwrap())
    };
    let lo = |b: [u8; 8]| u32::from_le_bytes(b[..4].try_into().unwrap());
    let mut out = Vec::new();
    let mut i = 0;
    for t in types {
        let Some(a) = slot(i) else { break };
        let value = match t {
            Ty::I64 => Value::I64(i64::from_le_bytes(a)),
            Ty::F32 => Value::F32(f32::from_bits(lo(a))),
            Ty::F64 => Value::F64(f64::from_le_bytes(a)),
            // A struct slot holds its own index into `chasm.refs`; the host reads it.
            Ty::Struct(name) => refs(i, name),
            Ty::Array(e) if matches!(e.as_ref(), Ty::Struct(_)) => {
                let Some(len) = slot(i + 2) else { break };
                Value::Opaque(format!("<{} elements>", lo(len)))
            }
            Ty::Str | Ty::Array(_) => {
                let Some(b) = slot(i + 1) else { break };
                let (addr, n) = (lo(a) as usize, lo(b) as usize);
                if matches!(t, Ty::Str) {
                    let bytes = mem.get(addr..addr.saturating_add(n)).unwrap_or(&[]);
                    Value::Str(String::from_utf8_lossy(bytes).into_owned())
                } else {
                    Value::Opaque(format!("<{n} elements>"))
                }
            }
            Ty::Quot(_) => Value::Opaque(format!("#{}", lo(a))),
            Ty::I32 | Ty::Var(_) => Value::I32(lo(a) as i32),
        };
        i += t.width();
        out.push(StackEntry {
            ty: t.to_string(),
            value: value.to_string(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation() {
        assert!(needs_more(": f ( -- i32 )"));
        assert!(!needs_more(": f ( -- i32 ) 1 ;"));
        assert!(needs_more("1 [ 2"));
        assert!(!needs_more("declare f ( -- )"));
        assert!(!needs_more("struct p  x: i32"));
        assert!(!needs_more("\"unterminated"));
    }

    #[test]
    fn stack_rendering() {
        let mut mem = vec![0u8; 0x10_0100];
        let base = layout::DATA_STACK_BASE as usize;
        mem[base..base + 4].copy_from_slice(&7i32.to_le_bytes());
        mem[base + 8..base + 12].copy_from_slice(&0x10_0000u32.to_le_bytes());
        mem[base + 16..base + 20].copy_from_slice(&2u32.to_le_bytes());
        mem[0x10_0000..0x10_0002].copy_from_slice(b"hi");
        let e = |ty: &str, value: &str| StackEntry {
            ty: ty.into(),
            value: value.into(),
        };
        assert_eq!(
            read_stack(&mem, &[Ty::I32, Ty::Str], &mut |_, _| panic!("no refs")),
            vec![e("i32", "7"), e("str", "\"hi\"")]
        );
        let point = Value::Struct {
            name: "point".into(),
            fields: vec![("x".into(), Value::I32(7)), ("y".into(), Value::F64(2.5))],
        };
        assert_eq!(
            read_stack(&mem, &[Ty::Struct("point".into())], &mut |i, n| {
                assert_eq!((i, n), (0, "point"));
                point.clone()
            }),
            vec![e("point", "point{x: 7, y: 2.5}")]
        );
        assert_eq!(
            read_stack(&mem, &[Ty::Struct("point".into())], &mut |_, _| Value::Null),
            vec![e("point", "null")]
        );
        let mut views = vec![0u8; 0x10_0100];
        for (k, v) in [5u32, 1, 2].iter().enumerate() {
            views[base + 8 * k..base + 8 * k + 4].copy_from_slice(&v.to_le_bytes());
        }
        assert_eq!(
            read_stack(
                &views,
                &[Ty::Array(Box::new(Ty::Struct("point".into())))],
                &mut |_, _| panic!("no refs")
            ),
            vec![e("array point", "<2 elements>")]
        );
        assert_eq!(stack_bytes(&[Ty::I32, Ty::Str]), 24);
    }
}
