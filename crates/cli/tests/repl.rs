//! Drive `chasm repl` through a pipe.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn repl(args: &[&str], input: &str) -> (bool, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_chasm"))
        .arg("repl")
        .args(args)
        .current_dir(root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run chasm repl");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const SESSION: &str = ": sq ( i32 -- i32 ) dup i32.mul ;\n3 sq\n\"hi\" println\n";

#[test]
fn text_session() {
    let (ok, out, err) = repl(&[], SESSION);
    assert!(ok, "{out}{err}");
    assert!(out.contains("ok: sq ( i32 -- i32 )\n"), "{out}");
    assert!(out.contains("( i32 ) 9\n"));
    assert!(out.lines().any(|l| l == "hi"));
    assert_eq!(out.lines().last(), Some("( i32 ) 9"));
}

#[test]
fn json_session() {
    let (ok, out, err) = repl(&["--json"], SESSION);
    assert!(ok, "{out}{err}");
    let reports: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(reports.len(), 3);
    for r in &reports {
        assert_eq!(r["schema"], 1);
        assert_eq!(r["command"], "repl");
    }
    assert_eq!(reports[1]["results"]["stack"][0]["type"], "i32");
    assert_eq!(reports[1]["results"]["stack"][0]["value"], "9");
    assert!(reports[1]["results"]["timing"]["compile_us"].is_u64());
    assert_eq!(reports[2]["results"]["output"], "hi\n");
}

#[test]
fn trap_fails_and_keeps_the_stack() {
    let (ok, out, err) = repl(&[], "1 0 i32.div_s\n");
    assert!(!ok);
    assert!(err.contains("trap in"), "{err}");
    assert_eq!(out.lines().last(), Some("( )"));
}

#[test]
fn definitions_continue_over_lines() {
    let (ok, out, err) = repl(&[], ": f ( -- i32 )\n  1 ;\nf\n");
    assert!(ok, "{out}{err}");
    assert!(out.contains("( i32 ) 1\n"), "{out}");
}

#[test]
fn tests_run_at_once() {
    let (ok, out, err) = repl(
        &[],
        ": sq ( i32 -- i32 ) dup i32.mul ;\ntest sq : 3 sq -> 9\n",
    );
    assert!(ok, "{out}{err}");
    assert!(out.contains("PASS"), "{out}");
}
