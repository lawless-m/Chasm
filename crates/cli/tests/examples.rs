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
