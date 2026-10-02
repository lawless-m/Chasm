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

const POINT: &str = "struct point  x: i32  y: f64\n";

fn last_line(out: &str) -> &str {
    out.lines().last().unwrap_or("")
}

/// The last stack echo, which spans several lines when it holds a struct.
fn last_stack(out: &str) -> String {
    let lines: Vec<&str> = out.lines().collect();
    let start = lines
        .iter()
        .rposition(|l| *l == "(" || l.starts_with("( "))
        .unwrap_or(0);
    lines[start..].join("\n")
}

#[test]
fn struct_echo_and_fields() {
    let (ok, out, err) = repl(&[], &format!("{POINT}7 2.5 point.new\n"));
    assert!(ok, "{out}{err}");
    assert!(out.contains("ok: point.new ( i32 f64 -- point )"));
    assert_eq!(last_stack(&out), "(\npoint point{x: 7, y: 2.5}\n)");
    let (_, out, _) = repl(&[], &format!("{POINT}7 2.5 point.new\ndup point.x\n"));
    assert_eq!(last_stack(&out), "(\npoint point{x: 7, y: 2.5}\ni32 7\n)");
    let (_, out, _) = repl(
        &[],
        &format!("{POINT}7 2.5 point.new\ndup point.x\ndrop dup 9 point.x!\n"),
    );
    assert_eq!(last_stack(&out), "(\npoint point{x: 9, y: 2.5}\n)");
}

#[test]
fn struct_arrays_in_the_repl() {
    let (_, out, _) = repl(&[], &format!("{POINT}2 array.new ( array point )\n"));
    assert_eq!(last_line(&out), "( array point ) <2 elements>");
    let (_, out, _) = repl(
        &[],
        &format!(
            "{POINT}2 array.new ( array point )\ndup 0 1 1.0 point.new array.at!\ndup 0 array.at\n"
        ),
    );
    assert_eq!(
        last_stack(&out),
        "(\narray point <2 elements>\npoint point{x: 1, y: 1.0}\n)"
    );
    let (_, out, _) = repl(
        &[],
        &format!("{POINT}2 array.new ( array point )\ndup 0 1 1.0 point.new array.at!\ndup 0 array.at\ndrop 1 1 array.slice\n"),
    );
    assert_eq!(last_line(&out), "( array point ) <1 elements>");
}

#[test]
fn struct_survives_trap_and_gc() {
    let (ok, out, err) = repl(&[], &format!("{POINT}7 2.5 point.new\n1 0 i32.div_s\n"));
    assert!(!ok);
    assert!(err.contains("trap in"), "{err}");
    assert_eq!(last_stack(&out), "(\npoint point{x: 7, y: 2.5}\n)");
    let t = std::time::Instant::now();
    let (ok, out, err) = repl(
        &[],
        &format!(
            "{POINT}: churn ( i32 -- ) [ drop 1 2.5 point.new drop ] times ;\n7 2.5 point.new\n10000000 churn\n"
        ),
    );
    assert!(ok, "{out}{err}");
    assert_eq!(last_stack(&out), "(\npoint point{x: 7, y: 2.5}\n)");
    assert!(t.elapsed().as_secs() < 10, "{:?}", t.elapsed());
}

#[test]
fn nested_struct_echo() {
    let (_, out, _) = repl(
        &[],
        &format!(
            "{POINT}struct seg  a: point  b: point\n0 0.0 point.new 1 1.0 point.new seg.new\n"
        ),
    );
    assert_eq!(
        last_stack(&out),
        "(\nseg seg{a: point{x: 0, y: 0.0}, b: point{x: 1, y: 1.0}}\n)"
    );
    // A null link comes from an unset array element.
    let (_, out, _) = repl(
        &[],
        "struct node  v: i32  next: node\n: nil ( -- node ) 1 array.new ( array node ) 0 array.at ;\n1 nil node.new 2 swap node.new 3 swap node.new 4 swap node.new\n",
    );
    assert_eq!(
        last_stack(&out),
        "(\nnode node{v: 4, next: node{v: 3, next: node{v: 2, next: node{...}}}}\n)"
    );
    let (_, out, _) = repl(
        &[],
        &format!("{POINT}struct bag  items: array point\n2 array.new ( array point ) bag.new\n"),
    );
    assert_eq!(last_stack(&out), "(\nbag bag{items: <2 elements>}\n)");
}

#[test]
fn struct_echo_json() {
    let (_, out, _) = repl(&["--json"], &format!("{POINT}7 2.5 point.new\n"));
    let last: serde_json::Value = serde_json::from_str(last_line(&out)).unwrap();
    assert_eq!(
        last["results"]["stack"][0],
        serde_json::json!({"type": "point", "value": "point{x: 7, y: 2.5}"})
    );
}

#[test]
fn forget_keeps_the_stack_and_old_function_values() {
    let input = ": sq ( i32 -- i32 ) dup i32.mul ;\n: ticked ( -- [ i32 -- i32 ] ) 'sq ;\n3 ticked\n)forget sq\n)forget ticked\n)forget sq\n: sq ( i32 -- i32 ) 1 i32.add ;\ncall\n";
    let (ok, out, err) = repl(&[], input);
    assert!(!ok, "the refused forget is an error");
    assert!(err.contains("E_FORGET"), "{err}");
    assert!(err.contains("dependants: ticked"), "{err}");
    assert!(
        out.contains("forgot: ticked\n") && out.contains("forgot: sq\n"),
        "{out}"
    );
    assert_eq!(last_line(&out), "( i32 ) 9", "the old `sq` still runs");
}

const FORCE: &str = ": f ( -- i32 ) 1 ;\n: g ( -- ) f drop ;\n: h ( -- i32 ) f ;\n'f\n)force : f ( -- i64 ) 1 i64 ;\n\n)force : f ( -- i64 ) 1 i64 ;\n: h ( -- i32 ) f i32.wrap_i64 ;\n\ncall\n'f call\n";

#[test]
fn force_refuses_then_commits() {
    let (ok, out, err) = repl(&[], FORCE);
    assert!(!ok, "the first force is refused");
    assert!(
        err.contains("E_FORCE") && err.contains("dependants: h"),
        "{err}"
    );
    assert!(
        out.contains("forced: f ( -- i32 ) -> ( -- i64 )\n"),
        "{out}"
    );
    assert!(out.contains("rechecked: g\n"), "{out}");
    assert!(
        out.contains("( i32 ) 1\n"),
        "a function value taken before the force runs the old code: {out}"
    );
    assert_eq!(last_line(&out), "( i32 i64 ) 1 1 i64");
}

#[test]
fn force_json() {
    let (_, out, _) = repl(&["--json"], FORCE);
    let reports: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let forced = reports
        .iter()
        .find(|r| {
            r["results"]["forced"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
        })
        .expect("a report with a forced word");
    assert_eq!(forced["results"]["forced"][0]["to"], "( -- i64 )");
    assert_eq!(forced["results"]["rechecked"], serde_json::json!(["g"]));
}
