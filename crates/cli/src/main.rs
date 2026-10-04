//! `chasm`: check, build, run and test Chasm programs.
//!
//! Every command builds a JSON report `{ schema, ok, command, diagnostics,
//! results }`. With `--json` that report is printed; otherwise the text
//! output is rendered from the same JSON, so the two cannot drift.

mod lsp;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use chasm_core::{compile, Compilation, Diagnostic, Location, Options, Source};
use chasm_runtime::namespace::{Config, Console, Mount};
use chasm_runtime::native::{run_tests, Runner, TestStatus};
use chasm_runtime::repl::{NativeRepl, Outcome};
use clap::{Args, Parser, Subcommand};
use serde_json::{json, Value as J};

#[derive(Parser)]
#[command(
    name = "chasm",
    version,
    about = "Chasm: a typed concatenative language that compiles to WebAssembly"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args)]
struct Common {
    /// Source files (.chasm), processed in order.
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// Print the machine-readable JSON report.
    #[arg(long)]
    json: bool,
    /// Compile without the standard prelude.
    #[arg(long)]
    no_prelude: bool,
}

#[derive(Args)]
struct HostArgs {
    /// Mount a local directory or a 9p server at /mnt/name: `--mount name=DIR` or `--mount name=9p://host:port`.
    #[arg(long = "mount", value_name = "NAME=DIR")]
    mounts: Vec<String>,
    /// Do not expose the host filesystem as `/file`.
    #[arg(long)]
    no_file: bool,
    /// Do not expose the network as /net.
    #[arg(long)]
    no_net: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Check effects and types without running anything.
    Check(Common),
    /// Compile to a wasm module.
    Build {
        #[command(flatten)]
        common: Common,
        /// Output path (default: first file with .wasm extension).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Skip Binaryen's `wasm-opt`.
        #[arg(long)]
        no_opt: bool,
        /// Import WASI preview1 instead of the Chasm ring host and export _start.
        #[arg(long)]
        wasi: bool,
    },
    /// Build and run `main ( -- )`.
    Run {
        #[command(flatten)]
        common: Common,
        #[command(flatten)]
        host: HostArgs,
        /// Also run Binaryen's `wasm-opt -O3`, as `build` does (often slower under wasmtime).
        #[arg(long)]
        opt: bool,
    },
    /// Run the tests written beside each word.
    Test {
        #[command(flatten)]
        common: Common,
        #[command(flatten)]
        host: HostArgs,
    },
    /// List declared words that have no body yet: the to-do list.
    Unresolved(Common),
    /// List words that `main` and the `export` words never reach.
    Dead(Common),
    /// List un-annotated words with their inferred effects.
    Infer {
        #[command(flatten)]
        common: Common,
        /// Insert the inferred effects into the source files in place.
        #[arg(long)]
        write: bool,
    },
    /// List every word with its effect.
    Words(Common),
    /// List every primitive with its effect.
    Prims {
        #[arg(long)]
        json: bool,
    },
    /// What a word calls.
    Deps {
        /// The word.
        word: String,
        #[command(flatten)]
        common: Common,
        /// Include indirect dependencies.
        #[arg(long)]
        all: bool,
    },
    /// What calls a word.
    UsedBy {
        /// The word.
        word: String,
        #[command(flatten)]
        common: Common,
    },
    /// Interactive REPL: reads chunks from stdin, compiles and runs each.
    Repl {
        #[command(flatten)]
        host: HostArgs,
        /// Print one JSON report per input chunk.
        #[arg(long)]
        json: bool,
        /// Start without the standard prelude.
        #[arg(long)]
        no_prelude: bool,
    },
    /// Language server over stdio, for editors.
    Lsp,
}

struct Report {
    command: &'static str,
    ok: bool,
    diagnostics: Vec<Diagnostic>,
    results: J,
}

impl Report {
    fn to_json(&self) -> J {
        json!({
            "schema": 1,
            "ok": self.ok,
            "command": self.command,
            "diagnostics": self.diagnostics,
            "results": self.results,
        })
    }
}

/// Insert each inferred effect after its word's name in the source file,
/// leaving every other byte as it was. Returns the files changed.
#[allow(clippy::result_large_err)]
fn write_effects(words: &[&chasm_core::WordInfo]) -> Result<Vec<String>, Diagnostic> {
    let mut by_file: BTreeMap<&str, Vec<&chasm_core::WordInfo>> = BTreeMap::new();
    for w in words {
        by_file.entry(w.location.file.as_str()).or_default().push(w);
    }
    let io = |file: &str, e: std::io::Error| {
        Diagnostic::error(
            "E_IO",
            format!("cannot rewrite `{file}`: {e}"),
            Location::default(),
        )
    };
    let mut written = Vec::new();
    for (file, words) in by_file {
        let mut text = std::fs::read_to_string(file).map_err(|e| io(file, e))?;
        // (byte offset just after the name, text to insert), last first.
        let mut edits: Vec<(usize, String)> = words
            .iter()
            .filter_map(|w| {
                let l = &w.location;
                let line_start: usize = text
                    .split_inclusive('\n')
                    .take(l.line.saturating_sub(1) as usize)
                    .map(str::len)
                    .sum();
                let col = text[line_start..]
                    .char_indices()
                    .nth(l.column.saturating_sub(1) as usize)
                    .map(|(i, _)| line_start + i)?;
                text[col..]
                    .starts_with(&l.token)
                    .then(|| (col + l.token.len(), format!(" {}", w.effect)))
            })
            .collect();
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        for (at, insert) in &edits {
            text.insert_str(*at, insert);
        }
        std::fs::write(file, text).map_err(|e| io(file, e))?;
        written.push(file.to_string());
    }
    Ok(written)
}

/// The wasm features Chasm emits; `wasm-opt` may use no others.
const WASM_OPT_FEATURES: &[&str] = &[
    "--enable-multivalue",
    "--enable-reference-types",
    "--enable-gc",
    "--enable-bulk-memory",
    "--enable-sign-ext",
    "--enable-nontrapping-float-to-int",
    "--enable-mutable-globals",
];

/// Run Binaryen's `wasm-opt -O3` (or `$CHASM_WASM_OPT`) over a module. Returns
/// the module to use and, when optimisation was wanted but did not happen,
/// why. A result that does not validate is not used.
fn optimise(raw: &[u8], skip: bool) -> (Vec<u8>, Option<String>) {
    if skip {
        return (raw.to_vec(), None);
    }
    let tool = std::env::var("CHASM_WASM_OPT").unwrap_or_else(|_| "wasm-opt".to_string());
    let dir = std::env::temp_dir();
    let input = dir.join(format!("chasm-{}.wasm", std::process::id()));
    let output = dir.join(format!("chasm-{}.opt.wasm", std::process::id()));
    if let Err(e) = std::fs::write(&input, raw) {
        return (
            raw.to_vec(),
            Some(format!(
                "not optimised: cannot write `{}`: {e}",
                input.display()
            )),
        );
    }
    let ran = std::process::Command::new(&tool)
        .args(["-O3", "-g"])
        .args(WASM_OPT_FEATURES)
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .output();
    let result = match ran {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(format!(
            "not optimised: `{tool}` not found; install Binaryen 121 or later, or pass --no-opt"
        )),
        Err(e) => Err(format!("not optimised: cannot run `{tool}`: {e}")),
        Ok(o) if !o.status.success() => Err(format!(
            "not optimised: `{tool}` failed: {}",
            String::from_utf8_lossy(&o.stderr)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
        )),
        Ok(_) => match std::fs::read(&output) {
            Ok(bytes) => match chasm_core::validate(&bytes) {
                Ok(()) => Ok(bytes),
                Err(e) => Err(format!(
                    "not optimised: `{tool}` produced an invalid module: {e}"
                )),
            },
            Err(e) => Err(format!(
                "not optimised: cannot read `{}`: {e}",
                output.display()
            )),
        },
    };
    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&output);
    match result {
        Ok(bytes) => (bytes, None),
        Err(note) => (raw.to_vec(), Some(note)),
    }
}

#[allow(clippy::result_large_err)]
fn load(
    c: &Common,
    test_exports: bool,
    export: bool,
    wasi: bool,
) -> Result<Compilation, Diagnostic> {
    let mut sources = Vec::new();
    for f in &c.files {
        let name = f.display().to_string();
        match std::fs::read_to_string(f) {
            Ok(text) => sources.push(Source::new(name, text)),
            Err(e) => {
                return Err(Diagnostic::error(
                    "E_IO",
                    format!("cannot read `{name}`: {e}"),
                    Location {
                        file: name,
                        ..Location::default()
                    },
                ))
            }
        }
    }
    Ok(compile(
        &sources,
        &Options {
            prelude: !c.no_prelude,
            test_exports,
            export,
            wasi,
        },
    ))
}

fn host_config(h: &HostArgs) -> Result<Config, String> {
    let mut mounts = BTreeMap::new();
    for m in &h.mounts {
        let (name, dir) = m
            .split_once('=')
            .ok_or_else(|| format!("bad --mount `{m}`: expected NAME=DIR"))?;
        let name = name.trim_start_matches("/mnt/").trim_matches('/');
        if name.is_empty() || name.contains('/') {
            return Err(format!(
                "bad --mount `{m}`: name must be a single path segment"
            ));
        }
        let mount = match dir.strip_prefix("9p://") {
            Some(addr) => {
                let port_ok = addr
                    .rsplit_once(':')
                    .is_some_and(|(host, port)| !host.is_empty() && port.parse::<u16>().is_ok());
                if !port_ok {
                    return Err(format!(
                        "bad --mount `{m}`: a 9p source is `9p://host:port`"
                    ));
                }
                Mount::NineP(addr.to_string())
            }
            None => Mount::Dir(PathBuf::from(dir)),
        };
        mounts.insert(name.to_string(), mount);
    }
    Ok(Config {
        console: Console::Std,
        file: !h.no_file,
        mounts,
        net: !h.no_net,
    })
}

fn words_json(c: &Compilation) -> J {
    json!(c.words)
}

fn failed(command: &'static str, diagnostics: Vec<Diagnostic>) -> Report {
    Report {
        command,
        ok: false,
        diagnostics,
        results: json!({}),
    }
}

fn exec(cli: Cli) -> (Report, bool) {
    match cli.cmd {
        Cmd::Check(c) => {
            let json = c.json;
            let r = match load(&c, false, false, false) {
                Err(d) => failed("check", vec![d]),
                Ok(comp) => Report {
                    command: "check",
                    ok: comp.ok(),
                    results: json!({
                        "words": comp.words.iter().filter(|w| !w.library).count(),
                        "library_words": comp.words.iter().filter(|w| w.library).count(),
                        "tests": comp.tests.len(),
                        "unresolved": comp.unresolved().iter().map(|w| &w.name).collect::<Vec<_>>(),
                        "has_main": comp.has_main,
                    }),
                    diagnostics: comp.diagnostics,
                },
            };
            (r, json)
        }
        Cmd::Build {
            common,
            output,
            no_opt,
            wasi,
        } => {
            let json = common.json;
            let r = match load(&common, false, true, wasi) {
                Err(d) => failed("build", vec![d]),
                Ok(comp) => match &comp.wasm {
                    None => failed("build", comp.diagnostics),
                    Some(raw) => {
                        let (bytes, note) = optimise(raw, no_opt);
                        let out = output.unwrap_or_else(|| common.files[0].with_extension("wasm"));
                        match std::fs::write(&out, &bytes) {
                            Ok(()) => Report {
                                command: "build",
                                ok: true,
                                results: json!({
                                    "output": out.display().to_string(),
                                    "bytes": bytes.len(),
                                    "unoptimised_bytes": raw.len(),
                                    "optimised": !no_opt && note.is_none(),
                                    "note": note,
                                    "wasi": wasi,
                                }),
                                diagnostics: comp.diagnostics,
                            },
                            Err(e) => failed(
                                "build",
                                vec![Diagnostic::error(
                                    "E_IO",
                                    format!("cannot write `{}`: {e}", out.display()),
                                    Location::default(),
                                )],
                            ),
                        }
                    }
                },
            };
            (r, json)
        }
        Cmd::Run { common, host, opt } => {
            let json = common.json;
            let mut cfg = match host_config(&host) {
                Ok(c) => c,
                Err(m) => {
                    return (
                        failed(
                            "run",
                            vec![Diagnostic::error("E_USAGE", m, Location::default())],
                        ),
                        json,
                    )
                }
            };
            let comp = match load(&common, false, true, false) {
                Ok(c) => c,
                Err(d) => return (failed("run", vec![d]), json),
            };
            let Some(wasm) = comp.wasm.as_ref() else {
                return (failed("run", comp.diagnostics), json);
            };
            if !comp.has_main {
                let d = Diagnostic::error(
                    "E_NO_MAIN",
                    "nothing to run: define `: main ( -- ) ... ;`",
                    Location {
                        file: common.files[0].display().to_string(),
                        ..Location::default()
                    },
                );
                return (failed("run", vec![d]), json);
            }
            if json {
                cfg.console = Console::Capture {
                    input: Vec::new(),
                    pos: 0,
                    output: Vec::new(),
                };
            }
            let (wasm, note) = optimise(wasm, !opt);
            let runner = match Runner::new(&wasm) {
                Ok(r) => r,
                Err(m) => {
                    return (
                        failed(
                            "run",
                            vec![Diagnostic::error("E_INTERNAL", m, Location::default())],
                        ),
                        json,
                    )
                }
            };
            let o = runner.run_main(cfg);
            let output = String::from_utf8_lossy(o.host.captured_output()).into_owned();
            let r = match o.result {
                Ok(()) => Report {
                    command: "run",
                    ok: true,
                    results: json!({ "output": output, "trap": null, "optimised": opt && note.is_none(), "note": note }),
                    diagnostics: comp.diagnostics,
                },
                Err(e) => Report {
                    command: "run",
                    ok: false,
                    results: json!({ "output": output, "trap": { "message": e.message, "word": e.word }, "optimised": opt && note.is_none(), "note": note }),
                    diagnostics: comp.diagnostics,
                },
            };
            (r, json)
        }
        Cmd::Test { common, host } => {
            let json = common.json;
            let cfg = match host_config(&host) {
                Ok(c) => c,
                Err(m) => {
                    return (
                        failed(
                            "test",
                            vec![Diagnostic::error("E_USAGE", m, Location::default())],
                        ),
                        json,
                    )
                }
            };
            let comp = match load(&common, true, false, false) {
                Ok(c) => c,
                Err(d) => return (failed("test", vec![d]), json),
            };
            if !comp.ok() {
                return (failed("test", comp.diagnostics), json);
            }
            let results = match run_tests(&comp, &cfg) {
                Ok(r) => r,
                Err(m) => {
                    return (
                        failed(
                            "test",
                            vec![Diagnostic::error("E_INTERNAL", m, Location::default())],
                        ),
                        json,
                    )
                }
            };
            let mut counts = BTreeMap::from([("pass", 0), ("fail", 0), ("pending", 0)]);
            let tests: Vec<J> = results
                .iter()
                .map(|r| {
                    *counts.get_mut(r.status.as_str()).unwrap() += 1;
                    json!({
                        "test": r.test.index,
                        "word": r.test.word,
                        "status": r.status.as_str(),
                        "expected": r.test.expected.iter().map(|v| v.to_string()).collect::<Vec<_>>(),
                        "actual": r.actual.as_ref().map(|a| a.iter().map(|v| v.to_string()).collect::<Vec<_>>()),
                        "trap": r.error.as_ref().map(|e| json!({ "message": e.message, "word": e.word })),
                        "output": String::from_utf8_lossy(&r.output),
                        "location": r.test.location,
                    })
                })
                .collect();
            let r = Report {
                command: "test",
                ok: counts["fail"] == 0,
                results: json!({ "tests": tests, "summary": counts }),
                diagnostics: comp.diagnostics,
            };
            (r, json)
        }
        Cmd::Unresolved(c) => {
            let json = c.json;
            let r = match load(&c, false, false, false) {
                Err(d) => failed("unresolved", vec![d]),
                Ok(comp) => {
                    let list: Vec<J> = comp
                        .unresolved()
                        .iter()
                        .map(|w| {
                            json!({
                                "word": w.name,
                                "declared_effect": w.effect,
                                "dependants": comp.graph.callers(&w.name),
                                "pending_tests": comp.tests.iter().filter(|t| t.word == w.name).count(),
                                "location": w.location,
                            })
                        })
                        .collect();
                    Report {
                        command: "unresolved",
                        ok: comp.ok(),
                        results: json!({ "unresolved": list }),
                        diagnostics: comp.diagnostics,
                    }
                }
            };
            (r, json)
        }
        Cmd::Infer { common, write } => {
            let json = common.json;
            let r = match load(&common, false, false, false) {
                Err(d) => failed("infer", vec![d]),
                Ok(comp) => {
                    let mut inferred: Vec<&chasm_core::WordInfo> = comp
                        .words
                        .iter()
                        .filter(|w| {
                            w.inferred && !w.library && !w.generated && w.instance_of.is_none()
                        })
                        .collect();
                    inferred.sort_by(|a, b| {
                        (&a.location.file, a.location.line, a.location.column).cmp(&(
                            &b.location.file,
                            b.location.line,
                            b.location.column,
                        ))
                    });
                    let words: Vec<J> = inferred
                        .iter()
                        .map(|w| json!({ "name": w.name, "effect": w.effect, "location": w.location }))
                        .collect();
                    let mut written = Vec::new();
                    let mut diagnostics = comp.diagnostics.clone();
                    if write && comp.ok() {
                        match write_effects(&inferred) {
                            Ok(files) => written = files,
                            Err(d) => diagnostics.push(d),
                        }
                    }
                    let ok = !diagnostics.iter().any(Diagnostic::is_error);
                    Report {
                        command: "infer",
                        ok,
                        results: json!({ "words": words, "written": written }),
                        diagnostics,
                    }
                }
            };
            (r, json)
        }
        Cmd::Dead(c) => {
            let json = c.json;
            let r = match load(&c, false, false, false) {
                Err(d) => failed("dead", vec![d]),
                Ok(comp) => {
                    let dead = comp.dead().map(|ws| {
                        ws.iter()
                            .map(|w| {
                                json!({
                                    "word": w.name,
                                    "effect": w.effect,
                                    "location": w.location,
                                })
                            })
                            .collect::<Vec<J>>()
                    });
                    Report {
                        command: "dead",
                        ok: comp.ok(),
                        results: json!({ "has_roots": dead.is_some(), "dead": dead.unwrap_or_default() }),
                        diagnostics: comp.diagnostics,
                    }
                }
            };
            (r, json)
        }
        Cmd::Words(c) => {
            let json = c.json;
            let r = match load(&c, false, false, false) {
                Err(d) => failed("words", vec![d]),
                Ok(comp) => Report {
                    command: "words",
                    ok: comp.ok(),
                    results: json!({ "words": words_json(&comp) }),
                    diagnostics: comp.diagnostics,
                },
            };
            (r, json)
        }
        Cmd::Prims { json } => {
            let primitives: Vec<J> = chasm_core::prims::names()
                .map(|name| {
                    let effect = lsp::primitive_effect(name)
                        .unwrap_or_else(|| panic!("primitive `{name}` has no effect"));
                    json!({ "name": name, "effect": effect })
                })
                .collect();
            let r = Report {
                command: "prims",
                ok: true,
                diagnostics: vec![],
                results: json!({ "primitives": primitives }),
            };
            (r, json)
        }
        Cmd::Deps { word, common, all } => {
            let json = common.json;
            let r = graph_query("deps", &word, &common, |comp| {
                if all {
                    let mut set = comp.graph.reachable([word.as_str()]);
                    set.remove(&word);
                    json!(set.into_iter().collect::<Vec<_>>())
                } else {
                    json!(comp.graph.callees(&word))
                }
            });
            (r, json)
        }
        Cmd::Repl { .. } => unreachable!("the REPL streams; handled in main"),
        Cmd::Lsp => unreachable!("the language server streams; handled in main"),
        Cmd::UsedBy { word, common } => {
            let json = common.json;
            let r = graph_query("used-by", &word, &common, |comp| {
                json!(comp.graph.callers(&word))
            });
            (r, json)
        }
    }
}

fn repl_report(o: Outcome, output: Vec<u8>) -> Report {
    let failed_test = o.tests.iter().any(|t| t.status == TestStatus::Fail);
    let trap =
        |e: &chasm_runtime::native::RunError| json!({ "message": e.message, "word": e.word });
    let results = json!({
        "defined": o.defined.iter().map(|d| json!({
            "name": d.name, "effect": d.effect, "declared": d.declared, "inferred": d.inferred,
        })).collect::<Vec<_>>(),
        "forgotten": o.forgotten,
        "forced": o.forced.iter().map(|f| json!({ "name": f.name, "from": f.from, "to": f.to })).collect::<Vec<_>>(),
        "rechecked": o.rechecked,
        "listing": o.listing,
        "tested": o.tested,
        "tests": o.tests.iter().map(|t| json!({
            "word": t.word,
            "status": t.status.as_str(),
            "expected": t.expected.iter().map(|v| v.to_string()).collect::<Vec<_>>(),
            "actual": t.actual.as_ref().map(|a| a.iter().map(|v| v.to_string()).collect::<Vec<_>>()),
            "trap": t.error.as_ref().map(trap),
            "location": t.location,
        })).collect::<Vec<_>>(),
        "trap": o.trap.as_ref().map(trap),
        "stack": o.stack.iter().map(|e| json!({ "type": e.ty, "value": e.value })).collect::<Vec<_>>(),
        "output": String::from_utf8_lossy(&output),
        "timing": o.timing,
    });
    Report {
        command: "repl",
        ok: !o.diagnostics.iter().any(Diagnostic::is_error) && o.trap.is_none() && !failed_test,
        diagnostics: o.diagnostics,
        results,
    }
}

/// The REPL: read chunks from stdin (continuing while a definition or
/// quotation is open), step each, and report. In text mode the program's
/// console is the terminal; with `--json` it is captured into the report.
fn run_repl(host: HostArgs, json: bool, no_prelude: bool) -> ExitCode {
    use std::io::{BufRead, IsTerminal, Write};
    let fail = |m: String, code: &str| {
        let r = failed(
            "repl",
            vec![Diagnostic::error(code, m, Location::default())],
        );
        print_report(&r, json);
        ExitCode::FAILURE
    };
    let mut cfg = match host_config(&host) {
        Ok(c) => c,
        Err(m) => return fail(m, "E_USAGE"),
    };
    if json {
        cfg.console = Console::Capture {
            input: Vec::new(),
            pos: 0,
            output: Vec::new(),
        };
    }
    let mut repl = match NativeRepl::new(cfg, !no_prelude) {
        Ok(r) => r,
        Err(m) => return fail(m, "E_INTERNAL"),
    };
    let tty = std::io::stdin().is_terminal();
    let mut all_ok = true;
    let mut chunk = String::new();
    loop {
        if tty {
            print!("{}", if chunk.is_empty() { "> " } else { ". " });
            let _ = std::io::stdout().flush();
        }
        // Lock stdin per line only: the program's `read-line` shares it.
        let mut line = String::new();
        let eof = !matches!(std::io::stdin().lock().read_line(&mut line), Ok(n) if n > 0);
        chunk.push_str(&line);
        if !eof && chasm_core::repl::needs_more(&chunk) {
            continue;
        }
        if !chunk.trim().is_empty() {
            let o = repl.step(&chunk);
            let output = repl.host_mut().take_output();
            let r = repl_report(o, output);
            all_ok &= r.ok;
            print_report(&r, json);
        }
        chunk.clear();
        if eof {
            break;
        }
    }
    if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn graph_query(
    command: &'static str,
    word: &str,
    c: &Common,
    f: impl Fn(&Compilation) -> J,
) -> Report {
    match load(c, false, false, false) {
        Err(d) => failed(command, vec![d]),
        Ok(comp) => {
            if comp.word(word).is_none() {
                let mut diags = comp.diagnostics.clone();
                diags.push(Diagnostic::error(
                    "E_UNDEFINED",
                    format!("unknown word `{word}`"),
                    Location::default(),
                ));
                return failed(command, diags);
            }
            Report {
                command,
                ok: comp.ok(),
                results: json!({ "word": word, "words": f(&comp) }),
                diagnostics: comp.diagnostics,
            }
        }
    }
}

// ---------------------------------------------------------------- text rendering (from JSON)

fn s(v: &J) -> String {
    match v {
        J::String(s) => s.clone(),
        J::Null => String::new(),
        other => other.to_string(),
    }
}

fn strs(v: &J) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(s).collect())
        .unwrap_or_default()
}

fn render(report: &J) -> (String, String) {
    let mut out = String::new();
    let mut err = String::new();
    for d in report["diagnostics"].as_array().into_iter().flatten() {
        match serde_json::from_value::<Diagnostic>(d.clone()) {
            Ok(d) => err.push_str(&format!("{}\n", d.render())),
            Err(_) => err.push_str(&format!("{d}\n")),
        }
    }
    let r = &report["results"];
    let ok = report["ok"].as_bool().unwrap_or(false);
    match report["command"].as_str().unwrap_or("") {
        "check" if ok => {
            out.push_str(&format!("ok: {} words, {} tests", r["words"], r["tests"]));
            let un = strs(&r["unresolved"]);
            if !un.is_empty() {
                out.push_str(&format!(", {} unresolved ({})", un.len(), un.join(", ")));
            }
            out.push('\n');
        }
        "build" if ok => {
            out.push_str(&format!(
                "wrote {} ({} bytes)\n",
                s(&r["output"]),
                r["bytes"]
            ));
            if let Some(n) = r["note"].as_str() {
                err.push_str(&format!("note: {n}\n"));
            }
        }
        "run" => {
            if let Some(n) = r["note"].as_str() {
                err.push_str(&format!("note: {n}\n"));
            }
            // In text mode the program wrote straight to stdout; `output`
            // is only filled when the console was captured.
            out.push_str(&s(&r["output"]));
            if let Some(t) = r.get("trap").filter(|t| !t.is_null()) {
                match t["word"].as_str() {
                    Some(w) => err.push_str(&format!("trap in `{w}`: {}\n", s(&t["message"]))),
                    None => err.push_str(&format!("trap: {}\n", s(&t["message"]))),
                }
            }
        }
        "test" => {
            for t in r["tests"].as_array().into_iter().flatten() {
                let status = s(&t["status"]);
                let loc = &t["location"];
                let at = format!("{}:{}", s(&loc["file"]), loc["line"]);
                let label = format!("{} #{}", s(&t["word"]), t["test"]);
                match status.as_str() {
                    "pass" => out.push_str(&format!("PASS     {label}\n")),
                    "pending" => {
                        out.push_str(&format!("PENDING  {label}  ({at}: word has no body yet)\n"))
                    }
                    _ => {
                        out.push_str(&format!("FAIL     {label}  ({at})\n"));
                        out.push_str(&format!(
                            "    expected: {}\n",
                            strs(&t["expected"]).join(" ")
                        ));
                        if !t["actual"].is_null() {
                            out.push_str(&format!(
                                "    actual:   {}\n",
                                strs(&t["actual"]).join(" ")
                            ));
                        }
                        if !t["trap"].is_null() {
                            out.push_str(&format!(
                                "    trap in `{}`: {}\n",
                                s(&t["trap"]["word"]),
                                s(&t["trap"]["message"])
                            ));
                        }
                    }
                }
            }
            let sum = &r["summary"];
            if !sum.is_null() {
                out.push_str(&format!(
                    "{} passed, {} failed, {} pending\n",
                    sum["pass"], sum["fail"], sum["pending"]
                ));
            }
        }
        "unresolved" => {
            let list = r["unresolved"].as_array().cloned().unwrap_or_default();
            if list.is_empty() && ok {
                out.push_str("no unresolved words\n");
            }
            for u in list {
                out.push_str(&format!("{} {}", s(&u["word"]), s(&u["declared_effect"])));
                let deps = strs(&u["dependants"]);
                if !deps.is_empty() {
                    out.push_str(&format!("  used by: {}", deps.join(", ")));
                }
                let p = u["pending_tests"].as_u64().unwrap_or(0);
                if p > 0 {
                    out.push_str(&format!("  pending tests: {p}"));
                }
                out.push('\n');
            }
        }
        "infer" => {
            let list = r["words"].as_array().cloned().unwrap_or_default();
            if list.is_empty() && ok {
                out.push_str("no un-annotated words\n");
            }
            for w in list {
                out.push_str(&format!(
                    "{} {}  ({}:{})\n",
                    s(&w["name"]),
                    s(&w["effect"]),
                    s(&w["location"]["file"]),
                    w["location"]["line"]
                ));
            }
            for f in strs(&r["written"]) {
                out.push_str(&format!("wrote {f}\n"));
            }
        }
        "dead" => {
            let list = r["dead"].as_array().cloned().unwrap_or_default();
            if r["has_roots"] == J::Bool(false) {
                if ok {
                    out.push_str(
                        "no roots: dead words are counted from `main` and `export` words\n",
                    );
                }
            } else if list.is_empty() && ok {
                out.push_str("no dead words\n");
            }
            for w in list {
                out.push_str(&format!("{} {}\n", s(&w["word"]), s(&w["effect"])));
            }
        }
        "prims" => {
            for p in r["primitives"].as_array().into_iter().flatten() {
                out.push_str(&format!("{} {}\n", s(&p["name"]), s(&p["effect"])));
            }
        }
        "words" => {
            for w in r["words"].as_array().into_iter().flatten() {
                let mut flags = Vec::new();
                if w["library"].as_bool() == Some(true) {
                    flags.push("library".to_string());
                }
                if w["resolved"].as_bool() == Some(false) {
                    flags.push("unresolved".to_string());
                }
                if w["export"].as_bool() == Some(true) {
                    flags.push("export".to_string());
                }
                if w["raw"].as_bool() == Some(true) {
                    flags.push("raw".to_string());
                }
                if w["inferred"].as_bool() == Some(true) {
                    flags.push("inferred".to_string());
                }
                if w["generic"].as_bool() == Some(true) {
                    flags.push("generic".to_string());
                }
                if let Some(t) = w["instance_of"].as_str() {
                    flags.push(format!("instance of {t}"));
                }
                let flags = if flags.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", flags.join(", "))
                };
                out.push_str(&format!("{} {}{}\n", s(&w["name"]), s(&w["effect"]), flags));
            }
        }
        "repl" => {
            for d in r["defined"].as_array().into_iter().flatten() {
                let declared = if d["declared"].as_bool() == Some(true) {
                    " (declared)"
                } else if d["inferred"].as_bool() == Some(true) {
                    " (inferred)"
                } else {
                    ""
                };
                out.push_str(&format!(
                    "ok: {} {}{declared}\n",
                    s(&d["name"]),
                    s(&d["effect"])
                ));
            }
            for w in strs(&r["forgotten"]) {
                out.push_str(&format!("forgot: {w}\n"));
            }
            for f in r["forced"].as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "forced: {} {} -> {}\n",
                    s(&f["name"]),
                    s(&f["from"]),
                    s(&f["to"])
                ));
            }
            let rechecked = strs(&r["rechecked"]);
            if !rechecked.is_empty() {
                out.push_str(&format!("rechecked: {}\n", rechecked.join(", ")));
            }
            if let Some(listing) = r["listing"].as_str() {
                out.push_str(if listing.is_empty() {
                    "(no words)\n"
                } else {
                    listing
                });
            }
            let mut counts = [0; 3];
            for t in r["tests"].as_array().into_iter().flatten() {
                match s(&t["status"]).as_str() {
                    "pass" => {
                        counts[0] += 1;
                        out.push_str(&format!("PASS     {}\n", s(&t["word"])));
                        continue;
                    }
                    "pending" => {
                        counts[2] += 1;
                        out.push_str(&format!(
                            "PENDING  {}  (word has no body yet)\n",
                            s(&t["word"])
                        ));
                        continue;
                    }
                    _ => counts[1] += 1,
                }
                let loc = &t["location"];
                out.push_str(&format!(
                    "FAIL     {}  ({}:{})\n",
                    s(&t["word"]),
                    s(&loc["file"]),
                    loc["line"]
                ));
                out.push_str(&format!(
                    "    expected: {}\n",
                    strs(&t["expected"]).join(" ")
                ));
                if !t["actual"].is_null() {
                    out.push_str(&format!("    actual:   {}\n", strs(&t["actual"]).join(" ")));
                }
                if !t["trap"].is_null() {
                    out.push_str(&format!(
                        "    trap in `{}`: {}\n",
                        s(&t["trap"]["word"]),
                        s(&t["trap"]["message"])
                    ));
                }
            }
            if r["tested"] == true {
                out.push_str(&format!(
                    "{} passed, {} failed, {} pending\n",
                    counts[0], counts[1], counts[2]
                ));
            }
            if let Some(t) = r.get("trap").filter(|t| !t.is_null()) {
                match t["word"].as_str() {
                    Some(w) => err.push_str(&format!("trap in `{w}`: {}\n", s(&t["message"]))),
                    None => err.push_str(&format!("trap: {}\n", s(&t["message"]))),
                }
            }
            out.push_str(&s(&r["output"]));
            let stack = r["stack"].as_array().cloned().unwrap_or_default();
            let types: Vec<String> = stack.iter().map(|e| s(&e["type"])).collect();
            let values: Vec<String> = stack.iter().map(|e| s(&e["value"])).collect();
            // A struct or union value (`point{..}`, `shape.circle{..}`) gets
            // a line of its own.
            let has_struct = values
                .iter()
                .any(|v| v.ends_with('}') && !v.starts_with('"'));
            if stack.is_empty() {
                out.push_str("( )\n");
            } else if has_struct {
                out.push_str("(\n");
                for (t, v) in types.iter().zip(&values) {
                    out.push_str(&format!("{t} {v}\n"));
                }
                out.push_str(")\n");
            } else {
                out.push_str(&format!("( {} ) {}\n", types.join(" "), values.join(" ")));
            }
        }
        "deps" | "used-by" => {
            for w in r["words"].as_array().into_iter().flatten() {
                match w {
                    J::Object(_) => {
                        let kind = s(&w["kind"]);
                        if kind == "call" {
                            out.push_str(&format!("{}\n", s(&w["word"])));
                        } else {
                            out.push_str(&format!("{}  ({kind})\n", s(&w["word"])));
                        }
                    }
                    _ => out.push_str(&format!("{}\n", s(w))),
                }
            }
        }
        _ => {}
    }
    (out, err)
}

fn print_report(report: &Report, json: bool) {
    let j = report.to_json();
    // Ignore write errors (e.g. a closed pipe): the exit code still reports.
    use std::io::Write;
    if json {
        // The REPL streams one compact report per line.
        let text = if report.command == "repl" {
            serde_json::to_string(&j)
        } else {
            serde_json::to_string_pretty(&j)
        };
        let _ = writeln!(std::io::stdout(), "{}", text.unwrap());
        let _ = std::io::stdout().flush();
    } else {
        let (out, err) = render(&j);
        if report.command == "repl" {
            // Errors first, so each chunk ends with its stack line.
            let _ = write!(std::io::stderr(), "{err}");
            let _ = write!(std::io::stdout(), "{out}");
            let _ = std::io::stdout().flush();
        } else {
            let _ = write!(std::io::stdout(), "{out}");
            let _ = std::io::stdout().flush();
            let _ = write!(std::io::stderr(), "{err}");
        }
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if std::env::args().any(|a| a == "--json") => {
            let cmd = match std::env::args().nth(1).as_deref() {
                Some("check") => "check",
                Some("build") => "build",
                Some("run") => "run",
                Some("test") => "test",
                Some("unresolved") => "unresolved",
                Some("dead") => "dead",
                Some("infer") => "infer",
                Some("words") => "words",
                Some("prims") => "prims",
                Some("deps") => "deps",
                Some("used-by") => "used-by",
                Some("repl") => "repl",
                Some("lsp") => "lsp",
                _ => "chasm",
            };
            let d = Diagnostic::error("E_USAGE", e.to_string().trim(), Location::default());
            print_report(&failed(cmd, vec![d]), true);
            return ExitCode::FAILURE;
        }
        Err(e) => e.exit(),
    };
    if let Cmd::Lsp = cli.cmd {
        return match lsp::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(m) => {
                let d = Diagnostic::error("E_INTERNAL", m, Location::default());
                print_report(&failed("lsp", vec![d]), true);
                ExitCode::FAILURE
            }
        };
    }
    if let Cmd::Repl {
        host,
        json,
        no_prelude,
    } = cli.cmd
    {
        return run_repl(host, json, no_prelude);
    }
    let (report, json) = exec(cli);
    print_report(&report, json);
    if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
