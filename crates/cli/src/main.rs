//! `chasm`: check, build, run and test Chasm programs.
//!
//! Every command builds a JSON report `{ schema, ok, command, diagnostics,
//! results }`. With `--json` that report is printed; otherwise the text
//! output is rendered from the same JSON, so the two cannot drift.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use chasm_core::{compile, Compilation, Diagnostic, Location, Options, Source};
use chasm_runtime::namespace::{Config, Console};
use chasm_runtime::native::{run_tests, Runner};
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
    /// Mount a local directory: `--mount name=DIR` or `--mount /mnt/name=DIR`.
    #[arg(long = "mount", value_name = "NAME=DIR")]
    mounts: Vec<String>,
    /// Do not expose the host filesystem as `/file`.
    #[arg(long)]
    no_file: bool,
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
    },
    /// Build and run `main ( -- )`.
    Run {
        #[command(flatten)]
        common: Common,
        #[command(flatten)]
        host: HostArgs,
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
    /// List every word with its effect.
    Words(Common),
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

#[allow(clippy::result_large_err)]
fn load(c: &Common, test_exports: bool) -> Result<Compilation, Diagnostic> {
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
        if let Some(src) = dir.strip_prefix("9p://") {
            return Err(format!(
                "9p mounts are not supported yet (`{src}`); mount a local directory"
            ));
        }
        mounts.insert(name.to_string(), PathBuf::from(dir));
    }
    Ok(Config {
        console: Console::Std,
        file: !h.no_file,
        mounts,
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
            let r = match load(&c, false) {
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
        Cmd::Build { common, output } => {
            let json = common.json;
            let r = match load(&common, false) {
                Err(d) => failed("build", vec![d]),
                Ok(comp) => match &comp.wasm {
                    None => failed("build", comp.diagnostics),
                    Some(bytes) => {
                        let out = output.unwrap_or_else(|| common.files[0].with_extension("wasm"));
                        match std::fs::write(&out, bytes) {
                            Ok(()) => Report {
                                command: "build",
                                ok: true,
                                results: json!({ "output": out.display().to_string(), "bytes": bytes.len() }),
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
        Cmd::Run { common, host } => {
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
            let comp = match load(&common, false) {
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
            let runner = match Runner::new(wasm) {
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
                    results: json!({ "output": output, "trap": null }),
                    diagnostics: comp.diagnostics,
                },
                Err(e) => Report {
                    command: "run",
                    ok: false,
                    results: json!({ "output": output, "trap": { "message": e.message, "word": e.word } }),
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
            let comp = match load(&common, true) {
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
            let r = match load(&c, false) {
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
        Cmd::Words(c) => {
            let json = c.json;
            let r = match load(&c, false) {
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
        Cmd::UsedBy { word, common } => {
            let json = common.json;
            let r = graph_query("used-by", &word, &common, |comp| {
                json!(comp.graph.callers(&word))
            });
            (r, json)
        }
    }
}

fn graph_query(
    command: &'static str,
    word: &str,
    c: &Common,
    f: impl Fn(&Compilation) -> J,
) -> Report {
    match load(c, false) {
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
        "build" if ok => out.push_str(&format!(
            "wrote {} ({} bytes)\n",
            s(&r["output"]),
            r["bytes"]
        )),
        "run" => {
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
        "words" => {
            for w in r["words"].as_array().into_iter().flatten() {
                let mut flags = Vec::new();
                if w["library"].as_bool() == Some(true) {
                    flags.push("library");
                }
                if w["resolved"].as_bool() == Some(false) {
                    flags.push("unresolved");
                }
                if w["export"].as_bool() == Some(true) {
                    flags.push("export");
                }
                let flags = if flags.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", flags.join(", "))
                };
                out.push_str(&format!("{} {}{}\n", s(&w["name"]), s(&w["effect"]), flags));
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

fn main() -> ExitCode {
    let cli = Cli::parse();
    let (report, json) = exec(cli);
    let j = report.to_json();
    // Ignore write errors (e.g. a closed pipe): the exit code still reports.
    use std::io::Write;
    if json {
        let _ = writeln!(
            std::io::stdout(),
            "{}",
            serde_json::to_string_pretty(&j).unwrap()
        );
    } else {
        let (out, err) = render(&j);
        let _ = write!(std::io::stdout(), "{out}");
        let _ = write!(std::io::stderr(), "{err}");
    }
    if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
