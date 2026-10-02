//! Run the CLI over the shipped examples.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn chasm(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_chasm"))
        .args(args)
        .current_dir(root())
        .output()
        .expect("run chasm");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn every_example_checks_and_its_tests_pass() {
    for entry in std::fs::read_dir(root().join("examples")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("chasm") {
            continue;
        }
        let p = path
            .strip_prefix(root())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let (ok, out, err) = chasm(&["check", &p]);
        assert!(ok, "check {p} failed:\n{out}{err}");
        let (ok, out, err) = chasm(&["test", &p]);
        assert!(ok, "tests in {p} failed:\n{out}{err}");
    }
}

#[test]
fn hello_runs() {
    let (ok, out, _) = chasm(&["run", "examples/hello.chasm"]);
    assert!(ok);
    assert_eq!(out, "Hello from Chasm\n");
}

#[test]
fn json_report_shape() {
    let (ok, out, _) = chasm(&["run", "--json", "examples/hello.chasm"]);
    assert!(ok);
    let j: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["schema"], 1);
    assert_eq!(j["command"], "run");
    assert_eq!(j["ok"], true);
    assert_eq!(j["results"]["output"], "Hello from Chasm\n");
}

#[test]
fn unresolved_lists_contract_stubs() {
    let (ok, out, _) = chasm(&["unresolved", "--json", "examples/contract.chasm"]);
    assert!(ok);
    let j: serde_json::Value = serde_json::from_str(&out).unwrap();
    let list = j["results"]["unresolved"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["word"], "parse-int");
    assert_eq!(list[0]["declared_effect"], "( str -- i32 )");
    assert_eq!(list[0]["dependants"][0], "double-parsed");
}

#[test]
fn benchmarks_check_against_their_examples() {
    for t in ["sieve", "mandelbrot", "n-queens", "quicksort"] {
        let (ok, out, err) = chasm(&[
            "check",
            &format!("examples/{t}.chasm"),
            &format!("bench/{t}.chasm"),
        ]);
        assert!(ok, "bench/{t}.chasm: {out}{err}");
    }
}

#[test]
fn dead_lists_words_main_never_reaches() {
    let (ok, out, _) = chasm(&["dead", "--json", "examples/basics.chasm"]);
    assert!(ok);
    let j: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["results"]["has_roots"], true);
    let names: Vec<&str> = j["results"]["dead"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["word"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["abs", "hypot"]);
    let (_, out, _) = chasm(&["dead", "examples/hello.chasm"]);
    assert_eq!(out, "no dead words\n");
    let (_, out, _) = chasm(&["dead", "examples/contract.chasm"]);
    assert!(out.starts_with("no roots"), "{out}");
}

#[test]
fn files_example_reads_a_mount() {
    let (ok, out, err) = chasm(&["run", "examples/files.chasm", "--mount", "ex=examples"]);
    assert!(ok, "{err}");
    assert!(out.contains("hello.chasm\n"));
    assert!(out.contains("--- hello.chasm ---"));
    assert!(out.contains("time moves forward"));
}

#[test]
fn errors_are_reported_with_codes() {
    let dir = std::env::temp_dir().join(format!("chasm-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("bad.chasm");
    std::fs::write(&f, ": f ( i32 -- i32 ) dup ;\n").unwrap();
    let (ok, out, _) = chasm(&["check", "--json", f.to_str().unwrap()]);
    assert!(!ok);
    let j: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["diagnostics"][0]["code"], "E_EFFECT_MISMATCH");
    assert_eq!(j["diagnostics"][0]["expected"][0], "i32");
    assert_eq!(
        j["diagnostics"][0]["actual"],
        serde_json::json!(["i32", "i32"])
    );
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("chasm-test-{}-{name}", std::process::id()))
}

#[test]
fn build_optimises_with_binaryen() {
    let out = scratch("sieve.wasm");
    let out = out.to_str().unwrap();
    let (ok, json, err) = chasm(&["build", "--json", "examples/sieve.chasm", "-o", out]);
    assert!(ok, "{json}{err}");
    let j: serde_json::Value = serde_json::from_str(&json).unwrap();
    let r = &j["results"];
    assert_eq!(
        r["optimised"], true,
        "is Binaryen 121 or later installed? {r}"
    );
    assert!(r["bytes"].as_u64() < r["unoptimised_bytes"].as_u64());
    let (_, json, _) = chasm(&[
        "build",
        "--json",
        "--no-opt",
        "examples/sieve.chasm",
        "-o",
        out,
    ]);
    let j: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(j["results"]["optimised"], false);
    assert!(j["results"]["note"].is_null());
    let _ = std::fs::remove_file(out);
}

#[test]
fn build_without_binaryen_notes_it_and_still_writes() {
    let out = scratch("hello.wasm");
    let o = Command::new(env!("CARGO_BIN_EXE_chasm"))
        .args(["build", "examples/hello.chasm", "-o", out.to_str().unwrap()])
        .env("CHASM_WASM_OPT", "no-such-wasm-opt")
        .current_dir(root())
        .output()
        .unwrap();
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("not optimised"));
    assert!(out.exists());
    let _ = std::fs::remove_file(out);
}

#[test]
fn run_refuses_reachable_unresolved_words() {
    let src = scratch("stub.chasm");
    std::fs::write(
        &src,
        "declare later ( -- i32 )\n: main ( -- ) later drop ;\n",
    )
    .unwrap();
    let (ok, _, err) = chasm(&["run", src.to_str().unwrap()]);
    assert!(!ok);
    assert!(err.contains("E_UNRESOLVED"), "{err}");
    let _ = std::fs::remove_file(src);
}

#[test]
fn json_report_shape_for_every_command() {
    std::fs::create_dir_all(root().join("tmp")).unwrap();
    let runs: &[(&str, &[&str])] = &[
        ("check", &["check", "--json", "examples/basics.chasm"]),
        (
            "build",
            &[
                "build",
                "--json",
                "examples/hello.chasm",
                "-o",
                "tmp/shape.wasm",
                "--no-opt",
            ],
        ),
        ("run", &["run", "--json", "examples/hello.chasm"]),
        ("test", &["test", "--json", "examples/basics.chasm"]),
        (
            "unresolved",
            &["unresolved", "--json", "examples/contract.chasm"],
        ),
        ("dead", &["dead", "--json", "examples/basics.chasm"]),
        ("words", &["words", "--json", "examples/basics.chasm"]),
        (
            "deps",
            &["deps", "--json", "square", "examples/basics.chasm"],
        ),
        (
            "used-by",
            &["used-by", "--json", "square", "examples/basics.chasm"],
        ),
    ];
    for (command, args) in runs {
        let (_, out, err) = chasm(args);
        let j: serde_json::Value =
            serde_json::from_str(&out).unwrap_or_else(|e| panic!("{command}: {e}: {out}{err}"));
        assert_eq!(j["schema"], 1, "{command}");
        assert_eq!(j["command"], *command, "{command}");
        assert!(j["ok"].is_boolean(), "{command}");
        assert!(j["diagnostics"].is_array(), "{command}");
        assert!(j["results"].is_object(), "{command}");
    }
    let (ok, out, _) = chasm(&["check", "--json", "no-such-file.chasm"]);
    assert!(!ok);
    let j: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["schema"], 1);
    assert_eq!(j["ok"], false);
    assert_eq!(j["diagnostics"][0]["code"], "E_IO");
    assert!(j["results"].is_object());
    let _ = std::fs::remove_file(root().join("tmp/shape.wasm"));
}

#[test]
fn usage_errors_are_json_reports_under_json() {
    let (ok, out, _) = chasm(&["check", "--json"]);
    assert!(!ok);
    let j: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["schema"], 1);
    assert_eq!(j["command"], "check");
    assert_eq!(j["ok"], false);
    assert_eq!(j["diagnostics"][0]["code"], "E_USAGE");
    let (ok, _, _) = chasm(&["--version"]);
    assert!(ok);
}

#[test]
fn build_wasi() {
    std::fs::create_dir_all(root().join("tmp")).unwrap();
    let out = format!("tmp/hello-wasi-{}.wasm", std::process::id());
    let (ok, json, err) = chasm(&[
        "build",
        "--json",
        "--wasi",
        "--no-opt",
        "examples/hello.chasm",
        "-o",
        &out,
    ]);
    assert!(ok, "{json}{err}");
    let j: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(j["results"]["wasi"], true);
    assert!(root().join(&out).exists());
    let _ = std::fs::remove_file(root().join(&out));
}
