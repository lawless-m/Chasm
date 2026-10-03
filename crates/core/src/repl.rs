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
use crate::parser::{parse_repl_with, parse_with, ReplInput};
use crate::program::Value;
use crate::program::{process_item, register_names, validate, Program, PRELUDE, PRELUDE_NAME};
use crate::types::{names, width_all, Ty};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

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
    /// The effect was inferred, not written.
    pub inferred: bool,
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
    /// Words a `)force` command gave a new effect.
    pub forced: Vec<Forced>,
    /// Dependants a `)force` command re-checked from their stored source, in
    /// the `dependants` spelling (`quad`, `test one`), sorted.
    pub rechecked: Vec<String>,
}

/// The session state a `)force` restores when it is refused.
type Snapshot = (
    Ctx,
    Program,
    HashSet<usize>,
    HashMap<WordId, Item>,
    HashMap<usize, Item>,
);

/// A word whose effect `)force` changed, effects as written.
#[derive(Debug, Clone, PartialEq)]
pub struct Forced {
    pub name: String,
    pub from: String,
    pub to: String,
}

impl Step {
    pub fn ok(&self) -> bool {
        !self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// A union variant's name, fields, and the table slot of each field's reader.
pub type VariantLayout = (String, Vec<(String, Ty)>, Vec<Option<u32>>);

/// How a struct or union value is laid out, for rendering it.
#[derive(Debug, Clone, PartialEq)]
pub enum Layout {
    /// The fields, in order, and the table slot of each field's reader.
    Struct {
        fields: Vec<(String, Ty)>,
        get: Vec<Option<u32>>,
    },
    /// The union's name, the table slot of its `tag` word (at these type
    /// arguments), and each variant's fields with their readers' slots.
    Union {
        name: String,
        tag_slot: u32,
        variants: Vec<VariantLayout>,
    },
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
    /// The source item of each user word's current definition.
    sources: HashMap<WordId, Item>,
    /// The source item of each test, by test index.
    test_items: HashMap<usize, Item>,
}

impl Session {
    /// Whether any struct or union type is registered.
    pub fn has_structs(&self) -> bool {
        self.ctx.has_ref_types()
    }

    pub fn structs(&self) -> &[StructDef] {
        &self.ctx.structs
    }

    pub fn struct_fields(&self, name: &str) -> Option<&[(String, Ty)]> {
        let &k = self.ctx.struct_by_name.get(name)?;
        Some(&self.ctx.structs[k].fields)
    }

    /// How to read a value of the registered struct or union type named
    /// `ty` (`point`, `pair i32 str`, `option i32`), parameters substituted.
    pub fn layout_of(&self, ty: &str) -> Option<Layout> {
        let Ty::Struct(name, args) = self.ctx.registered.get(ty)? else {
            return None;
        };
        let subst = |params: &[String], fields: &[(String, Ty)]| -> Vec<(String, Ty)> {
            let map: HashMap<String, Ty> =
                params.iter().cloned().zip(args.iter().cloned()).collect();
            fields
                .iter()
                .map(|(f, t)| (f.clone(), t.substitute(&map)))
                .collect()
        };
        // The slot of a generated word at these type arguments.
        let slot = |w: String| -> Option<u32> {
            let id = *self.ctx.by_name.get(&w)?;
            let id = if args.is_empty() {
                id
            } else {
                *self.ctx.instances.get(&(id, args.clone()))?
            };
            Some(id as u32)
        };
        if let Some(&k) = self.ctx.struct_by_name.get(name) {
            let s = &self.ctx.structs[k];
            return Some(Layout::Struct {
                fields: subst(&s.params, &s.fields),
                get: s
                    .fields
                    .iter()
                    .map(|(f, _)| slot(format!("{name}.{f}")))
                    .collect(),
            });
        }
        let u = &self.ctx.unions[*self.ctx.union_by_name.get(name)?];
        Some(Layout::Union {
            name: name.clone(),
            tag_slot: slot(format!("{name}.tag"))?,
            variants: u
                .variants
                .iter()
                .map(|(v, f)| {
                    let get = f
                        .iter()
                        .map(|(x, _)| slot(format!("{name}.{v}.{x}")))
                        .collect();
                    (v.clone(), subst(&u.params, f), get)
                })
                .collect(),
        })
    }

    /// The display names of every registered struct and union type, sorted.
    pub fn type_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.ctx.registered.keys().cloned().collect();
        v.sort();
        v
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
            sources: HashMap::new(),
            test_items: HashMap::new(),
        };
        let mut built = Vec::new();
        if prelude {
            match lex(PRELUDE_NAME, PRELUDE)
                .and_then(|t| parse_with(PRELUDE_NAME, &t, &HashMap::new()))
            {
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
        let known = self.ctx.type_arities();
        match lex(&file, text).and_then(|t| parse_repl_with(&file, &t, &known)) {
            Err(d) => self.program.diagnostics.push(d),
            Ok(ReplInput::Items(mut items)) => {
                register_names(&mut self.ctx, &items);
                crate::infer::require_effects(&mut [&mut items], &mut self.program.diagnostics);
                for item in items {
                    let named = matches!(
                        item,
                        Item::Def { .. }
                            | Item::Declare { .. }
                            | Item::Struct { .. }
                            | Item::Union { .. }
                    );
                    let source = item.clone();
                    let tests = self.program.tests.len();
                    let ids = process_item(&mut self.ctx, item, Origin::User, &mut self.program);
                    if let (Item::Def { .. }, Some(&id)) = (&source, ids.first()) {
                        self.sources.insert(id, source.clone());
                    }
                    if matches!(source, Item::Test { .. }) && self.program.tests.len() > tests {
                        self.test_items.insert(tests, source);
                    }
                    for id in ids {
                        built.push(id);
                        if named && self.ctx.words[id].kind == WordKind::Named {
                            let w = &self.ctx.words[id];
                            defined.push(Defined {
                                name: w.name.clone(),
                                effect: w.effect.to_string(),
                                declared: w.body.is_none(),
                                inferred: w.inferred,
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
                            inferred: false,
                            generic: None,
                            instance_of: None,
                            generated: None,
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
        if let Some(rest) = text.strip_prefix("force") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return self.force(rest, heap_ptr);
            }
        }
        let mut forgotten = Vec::new();
        match text.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["forget", name] => match self.forget(name, &loc) {
                Ok(()) => forgotten.push(name.to_string()),
                Err(d) => self.program.diagnostics.push(d),
            },
            _ => self.program.diagnostics.push(Diagnostic::error(
                codes::E_SYNTAX,
                format!(
                    "unknown REPL command `){}`; the commands are `)forget word` and `)force` definitions",
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

    /// `)force`: change words' effects deliberately. The chunk's definitions
    /// and every dependant of a changed word are checked together; if any
    /// fails, nothing changes and `E_FORCE` lists what broke.
    fn force(&mut self, text: &str, heap_ptr: u32) -> Step {
        self.steps += 1;
        let file = format!("<repl:{}>", self.steps);
        let before = self.ctx.words.len();
        let tests_before = self.program.tests.len();
        let known = self.ctx.type_arities();
        let parsed = lex(&file, text).and_then(|t| parse_with(&file, &t, &known));
        let outcome = match parsed {
            Err(d) => Err(vec![d]),
            Ok(items) => self.force_items(items),
        };
        match outcome {
            Ok((built, defined, forced, rechecked)) => {
                let mut step = self.finish(heap_ptr, before, tests_before, built, defined, None);
                step.forced = forced;
                step.rechecked = rechecked;
                step
            }
            Err(diagnostics) => {
                self.program.diagnostics.extend(diagnostics);
                self.finish(heap_ptr, before, tests_before, Vec::new(), Vec::new(), None)
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn force_items(
        &mut self,
        mut items: Vec<Item>,
    ) -> Result<(Vec<WordId>, Vec<Defined>, Vec<Forced>, Vec<String>), Vec<Diagnostic>> {
        let mut needs = Vec::new();
        crate::infer::require_effects(&mut [&mut items], &mut needs);
        if !needs.is_empty() {
            return Err(needs);
        }
        // Refusals, before anything changes.
        let mut changed: Vec<(String, WordId)> = Vec::new();
        let mut in_chunk: HashSet<WordId> = HashSet::new();
        for item in &items {
            match item {
                Item::Def {
                    name,
                    effect,
                    body,
                    loc,
                    ..
                } => {
                    let id = self
                        .changeable(name, loc, codes::E_FORCE, "forced")
                        .map_err(|d| vec![d])?;
                    in_chunk.insert(id);
                    // An un-annotated definition is inferred first, to see
                    // whether the chunk changes the word's effect.
                    let effect = match effect {
                        Some(e) => e.clone(),
                        None => crate::infer::infer_effect(&mut self.ctx, name, body, loc)
                            .map_err(|d| vec![d.with_word(name)])?,
                    };
                    if effect != self.ctx.words[id].effect
                        && !changed.iter().any(|(n, _)| n == name)
                    {
                        changed.push((name.clone(), id));
                    }
                }
                Item::Test { .. } => {}
                Item::Declare { loc, .. } | Item::Struct { loc, .. } | Item::Union { loc, .. } => {
                    return Err(vec![Diagnostic::error(
                        codes::E_SYNTAX,
                        "`)force` takes definitions and tests; `declare`, `struct` and `union` are not allowed",
                        loc.clone(),
                    )]);
                }
            }
        }
        // Dependants of the changed words, on the graph before the change.
        let mut affected_words: Vec<WordId> = Vec::new();
        let mut affected_tests: Vec<usize> = Vec::new();
        for (name, id) in &changed {
            let (words, tests) = self.dependant_ids(*id, name);
            affected_words.extend(words.into_iter().filter(|w| !in_chunk.contains(w)));
            affected_tests.extend(tests);
        }
        affected_words.sort();
        affected_words.dedup();
        affected_tests.sort();
        affected_tests.dedup();
        let old_effects: Vec<String> = changed
            .iter()
            .map(|(_, id)| self.ctx.words[*id].effect.to_string())
            .collect();

        let snapshot = (
            self.ctx.clone(),
            self.program.clone(),
            self.forgotten_tests.clone(),
            self.sources.clone(),
            self.test_items.clone(),
        );
        let errors = |p: &Program| p.diagnostics.iter().filter(|d| d.is_error()).count();

        register_names(&mut self.ctx, &items);
        for (name, id) in &changed {
            self.ctx.by_name.remove(name);
            self.retire(*id, name, "forced");
        }
        let mut built = Vec::new();
        let mut defined = Vec::new();
        for item in items {
            let source = item.clone();
            let tests = self.program.tests.len();
            let ids = process_item(&mut self.ctx, item, Origin::User, &mut self.program);
            if let (Item::Def { .. }, Some(&id)) = (&source, ids.first()) {
                self.sources.insert(id, source.clone());
            }
            if matches!(source, Item::Test { .. }) && self.program.tests.len() > tests {
                self.test_items.insert(tests, source.clone());
            }
            for id in ids {
                built.push(id);
                if matches!(source, Item::Def { .. }) && self.ctx.words[id].kind == WordKind::Named
                {
                    let w = &self.ctx.words[id];
                    defined.push(Defined {
                        name: w.name.clone(),
                        effect: w.effect.to_string(),
                        declared: w.body.is_none(),
                        inferred: w.inferred,
                    });
                }
            }
        }
        let mut broken = Vec::new();
        let mut rechecked = Vec::new();
        for id in affected_words {
            let name = self.ctx.words[id].name.clone();
            let Some(source) = self.sources.get(&id).cloned() else {
                return Err(self.internal(snapshot, &name));
            };
            let errs = errors(&self.program);
            process_item(&mut self.ctx, source, Origin::User, &mut self.program);
            if errors(&self.program) > errs {
                broken.push(name.clone());
            }
            built.push(id);
            rechecked.push(name);
        }
        for index in affected_tests {
            let label = format!("test {}", self.program.tests[index].word);
            let Some(source) = self.test_items.get(&index).cloned() else {
                return Err(self.internal(snapshot, &label));
            };
            self.forgotten_tests.insert(index);
            let errs = errors(&self.program);
            let tests = self.program.tests.len();
            process_item(
                &mut self.ctx,
                source.clone(),
                Origin::User,
                &mut self.program,
            );
            if self.program.tests.len() > tests {
                self.test_items.insert(tests, source);
            }
            if errors(&self.program) > errs {
                broken.push(label.clone());
            }
            rechecked.push(label);
        }

        if errors(&self.program) > 0 {
            let diagnostics = std::mem::take(&mut self.program.diagnostics);
            self.restore(snapshot);
            broken.sort();
            broken.dedup();
            let (name, _) = &changed.first().cloned().unwrap_or_default();
            let what = if name.is_empty() {
                "this `)force`".to_string()
            } else {
                format!("`{name}`")
            };
            let message = if broken.is_empty() {
                format!("{what} does not check; nothing was changed")
            } else {
                format!(
                    "forcing {what} breaks {}; nothing was changed",
                    broken.join(", ")
                )
            };
            let loc = diagnostics
                .first()
                .map(|d| d.location.clone())
                .unwrap_or_default();
            let mut d = Diagnostic::error(codes::E_FORCE, message, loc);
            if !name.is_empty() {
                d = d.with_word(name);
                d.declared_effect = old_effects.first().cloned();
            }
            d.dependants = Some(broken);
            return Err(std::iter::once(d).chain(diagnostics).collect());
        }
        let forced = changed
            .iter()
            .zip(old_effects)
            .map(|((name, _), from)| Forced {
                name: name.clone(),
                from,
                to: self.ctx.words[self.ctx.by_name[name]].effect.to_string(),
            })
            .collect();
        rechecked.sort();
        rechecked.dedup();
        Ok((built, defined, forced, rechecked))
    }

    fn restore(&mut self, (ctx, program, forgotten, sources, tests): Snapshot) {
        self.ctx = ctx;
        self.program = program;
        self.forgotten_tests = forgotten;
        self.sources = sources;
        self.test_items = tests;
    }

    fn internal(&mut self, snapshot: Snapshot, what: &str) -> Vec<Diagnostic> {
        self.restore(snapshot);
        vec![Diagnostic::error(
            codes::E_INTERNAL,
            format!("no stored source for {what} (this is a compiler bug); nothing was changed"),
            Location::default(),
        )]
    }

    /// The id of a user word that may be forgotten or forced. Primitives,
    /// prelude words and struct-generated words are refused with `code`.
    fn changeable(
        &self,
        name: &str,
        loc: &Location,
        code: &str,
        verb: &str,
    ) -> Result<WordId, Diagnostic> {
        let refuse = |msg: String| Diagnostic::error(code, msg, loc.clone()).with_word(name);
        let Some(&id) = self.ctx.by_name.get(name) else {
            if crate::prims::is_builtin(name) {
                return Err(refuse(format!(
                    "`{name}` is a primitive and cannot be {verb}"
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
                "`{name}` is a prelude word and cannot be {verb}"
            )));
        }
        if let Some(t) = &self.ctx.words[id].generated {
            let kind = if self.ctx.union_by_name.contains_key(t) {
                "union"
            } else {
                "struct"
            };
            return Err(refuse(format!(
                "`{name}` is generated by {kind} `{t}` and cannot be {verb} on its own"
            )));
        }
        Ok(id)
    }

    /// Remove a user word and its tests, freeing the name. Refused while any
    /// word or another word's test uses it. Its table slot is never reused,
    /// so a function value of it still on the stack keeps calling its code.
    fn forget(&mut self, name: &str, loc: &Location) -> Result<(), Diagnostic> {
        let id = self.changeable(name, loc, codes::E_FORGET, "forgotten")?;
        let dependants = self.dependants(id, name);
        if !dependants.is_empty() {
            let mut d = Diagnostic::error(
                codes::E_FORGET,
                format!(
                    "`{name}` is still used by {}; forget those first",
                    dependants.join(", ")
                ),
                loc.clone(),
            )
            .with_word(name);
            d.dependants = Some(dependants);
            return Err(d);
        }
        self.ctx.by_name.remove(name);
        self.ctx.all_names.remove(name);
        self.retire(id, name, "forgotten");
        Ok(())
    }

    /// Detach word `id` from its name: it keeps its slot and code, loses its
    /// edges, and its tests never run again. A generic word's instances go
    /// with it (they keep their slots too), so later uses instantiate afresh.
    fn retire(&mut self, id: WordId, name: &str, how: &str) {
        let mut instances: Vec<((WordId, Vec<Ty>), WordId)> = self
            .ctx
            .instances
            .iter()
            .filter(|((t, _), _)| *t == id)
            .map(|(k, &v)| (k.clone(), v))
            .collect();
        instances.sort_by_key(|(_, v)| *v);
        for (key, inst) in instances {
            self.ctx.instances.remove(&key);
            let w = &mut self.ctx.words[inst];
            w.name = format!("[{how} {}]", w.name);
            w.callees.clear();
        }
        let w = &mut self.ctx.words[id];
        w.name = format!("[{how} {name}]");
        w.callees.clear();
        for t in &self.program.tests {
            if t.word == name {
                self.forgotten_tests.insert(t.index);
            }
        }
    }

    /// The named words, and the tests of other words, that use word `id`
    /// directly or through quotations. REPL lines do not count.
    fn dependants(&self, id: WordId, name: &str) -> Vec<String> {
        let (words, tests) = self.dependant_ids(id, name);
        let mut out: Vec<String> = words
            .iter()
            .map(|&w| self.ctx.words[w].name.clone())
            .chain(
                tests
                    .iter()
                    .map(|&t| format!("test {}", self.program.tests[t].word)),
            )
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// `dependants` as ids: named words, and indices of other words' tests.
    fn dependant_ids(&self, id: WordId, name: &str) -> (Vec<WordId>, Vec<usize>) {
        let test_of: HashMap<WordId, usize> = self
            .program
            .tests
            .iter()
            .filter(|t| !self.forgotten_tests.contains(&t.index) && t.word != name)
            .map(|t| (self.program.test_words[t.index], t.index))
            .collect();
        let mut seen = HashSet::from([id]);
        let mut todo = vec![id];
        let (mut words, mut tests) = (Vec::new(), Vec::new());
        while let Some(target) = todo.pop() {
            for (caller, w) in self.ctx.words.iter().enumerate() {
                if !w.callees.iter().any(|&(c, _)| c == target) || !seen.insert(caller) {
                    continue;
                }
                match w.kind {
                    WordKind::Named => words.push(caller),
                    WordKind::Quote | WordKind::Instance => todo.push(caller),
                    WordKind::Test => tests.extend(test_of.get(&caller)),
                    WordKind::Line => {}
                }
            }
        }
        (words, tests)
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
            forced: Vec::new(),
            rechecked: Vec::new(),
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
/// a `:` definition or a `[` quotation is still open, or a `)force` chunk
/// has not yet ended with an empty line. `text` holds each line with its
/// newline. A lex error is left for the step to report.
pub fn needs_more(text: &str) -> bool {
    let command = text.trim_start();
    if command.starts_with(")force") {
        let body = text.strip_suffix('\n').unwrap_or(text);
        let last = body.rsplit('\n').next().unwrap_or("");
        return !(body.contains('\n') && last.trim().is_empty());
    }
    if command.starts_with(')') {
        return false;
    }
    let Ok(toks) = lex("<repl>", text) else {
        return false;
    };
    if toks
        .first()
        .is_some_and(|t| t.is("declare") || t.is("test") || t.is("struct") || t.is("union"))
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
            Ty::Struct(..) => refs(i, &t.to_string()),
            Ty::Array(e) if matches!(e.as_ref(), Ty::Struct(..)) => {
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
            Ty::I32 | Ty::Var(_) | Ty::Param(_) => Value::I32(lo(a) as i32),
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
            read_stack(
                &mem,
                &[Ty::Struct("point".into(), Vec::new())],
                &mut |i, n| {
                    assert_eq!((i, n), (0, "point"));
                    point.clone()
                }
            ),
            vec![e("point", "point{x: 7, y: 2.5}")]
        );
        assert_eq!(
            read_stack(
                &mem,
                &[Ty::Struct("point".into(), Vec::new())],
                &mut |_, _| Value::Null
            ),
            vec![e("point", "null")]
        );
        let mut views = vec![0u8; 0x10_0100];
        for (k, v) in [5u32, 1, 2].iter().enumerate() {
            views[base + 8 * k..base + 8 * k + 4].copy_from_slice(&v.to_le_bytes());
        }
        assert_eq!(
            read_stack(
                &views,
                &[Ty::Array(Box::new(Ty::Struct("point".into(), Vec::new())))],
                &mut |_, _| panic!("no refs")
            ),
            vec![e("array point", "<2 elements>")]
        );
        assert_eq!(stack_bytes(&[Ty::I32, Ty::Str]), 24);
    }
}
