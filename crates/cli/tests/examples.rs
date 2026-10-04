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
        ("infer", &["infer", "--json", "examples/basics.chasm"]),
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

#[test]
fn generic_words_run_and_test() {
    let src = scratch("generic.chasm");
    std::fs::write(
        &src,
        ": twice ( T -- T T ) dup ;\n: main ( -- ) 3 twice i32.add i32.to-str println ;\ntest twice : 3 twice -> 3 3\ntest twice : \"a\" twice str.concat -> \"aa\"\n",
    )
    .unwrap();
    let p = src.to_str().unwrap();
    let (ok, out, err) = chasm(&["test", p]);
    assert!(ok, "{out}{err}");
    assert!(out.contains("2 passed"), "{out}");
    let (ok, out, err) = chasm(&["run", p]);
    assert!(ok, "{out}{err}");
    assert_eq!(out, "6\n");
    let _ = std::fs::remove_file(src);
}

#[test]
fn dead_and_build_with_generics() {
    let src = scratch("generic-dead.chasm");
    std::fs::write(
        &src,
        ": twice ( T -- T T ) dup ;\n: main ( -- ) 4 twice i32.mul i32.to-str println ;\n",
    )
    .unwrap();
    let p = src.to_str().unwrap();
    let (_, out, _) = chasm(&["dead", p]);
    assert!(!out.contains("twice"), "{out}");
    let wasm = scratch("generic-dead.wasm");
    let (ok, out, err) = chasm(&["build", "--no-opt", p, "-o", wasm.to_str().unwrap()]);
    assert!(ok, "{out}{err}");
    let (ok, out, _) = chasm(&["run", p]);
    assert!(ok);
    assert_eq!(out, "16\n");
    let _ = std::fs::remove_file(src);
    let _ = std::fs::remove_file(wasm);
}

#[test]
fn the_last_definition_of_a_generic_wins() {
    let src = scratch("generic-redef.chasm");
    std::fs::write(
        &src,
        ": pick2 ( T T -- T ) drop ;\n: f ( i32 i32 -- i32 ) pick2 ;\n: pick2 ( T T -- T ) nip ;\ntest f : 1 2 f -> 2\n",
    )
    .unwrap();
    let (ok, out, err) = chasm(&["test", src.to_str().unwrap()]);
    assert!(ok, "{out}{err}");
    let _ = std::fs::remove_file(src);
}

#[test]
fn words_flags_inferred_and_generic() {
    let src = scratch("words-flags.chasm");
    std::fs::write(
        &src,
        ": sq dup i32.mul ;\n: twice ( T -- T T ) dup ;\n: a ( i32 -- i32 i32 ) twice ;\n",
    )
    .unwrap();
    let p = src.to_str().unwrap();
    let (_, out, _) = chasm(&["words", "--json", p]);
    let j: serde_json::Value = serde_json::from_str(&out).unwrap();
    let word = |n: &str| {
        j["results"]["words"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["name"] == n)
            .cloned()
            .unwrap()
    };
    assert_eq!(word("sq")["inferred"], true);
    assert_eq!(word("twice")["generic"], true);
    assert_eq!(word("twice<i32>")["instance_of"], "twice");
    let (_, out, _) = chasm(&["words", p]);
    assert!(out.contains("sq ( i32 -- i32 )  [inferred]"), "{out}");
    assert!(out.contains("twice ( T -- T T )  [generic]"), "{out}");
    let _ = std::fs::remove_file(src);
}

#[test]
fn infer_lists_unannotated_words() {
    let src = scratch("infer.chasm");
    std::fs::write(
        &src,
        ": sq dup i32.mul ;\n: twice dup ;\n: main ( -- ) 3 sq twice i32.add i32.to-str println ;\n",
    )
    .unwrap();
    let p = src.to_str().unwrap();
    let (ok, out, err) = chasm(&["infer", "--json", p]);
    assert!(ok, "{out}{err}");
    let j: serde_json::Value = serde_json::from_str(&out).unwrap();
    let words = j["results"]["words"].as_array().unwrap();
    assert_eq!(words.len(), 2, "{out}");
    assert_eq!(
        (words[0]["name"].as_str(), words[0]["effect"].as_str()),
        (Some("sq"), Some("( i32 -- i32 )"))
    );
    assert_eq!(words[0]["location"]["line"], 1);
    assert_eq!(
        (words[1]["name"].as_str(), words[1]["effect"].as_str()),
        (Some("twice"), Some("( T -- T T )"))
    );
    let (_, out, _) = chasm(&["infer", p]);
    assert!(
        out.contains("sq ( i32 -- i32 )") && out.contains("twice ( T -- T T )"),
        "{out}"
    );
    std::fs::write(&src, ": bad 1 \"x\" i32.add ;\n").unwrap();
    let (ok, out, _) = chasm(&["infer", "--json", p]);
    assert!(!ok);
    let j: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["diagnostics"][0]["code"], "E_TYPE_MISMATCH");
    let _ = std::fs::remove_file(src);
}

#[test]
fn infer_write_inserts_effects() {
    let src = scratch("infer-write.chasm");
    let before = "# header\n: sq dup i32.mul ;\n: twice dup ;\n:  spaced   ( i32 -- i32 ) 1 i32.add ;\n: main ( -- ) 3 sq twice i32.add i32.to-str println ;\n";
    std::fs::write(&src, before).unwrap();
    let p = src.to_str().unwrap();
    let (ok, out, err) = chasm(&["infer", "--write", p]);
    assert!(ok, "{out}{err}");
    let after = std::fs::read_to_string(&src).unwrap();
    assert_eq!(
        after,
        before
            .replace(": sq dup", ": sq ( i32 -- i32 ) dup")
            .replace(": twice dup", ": twice ( T -- T T ) dup")
    );
    assert!(chasm(&["check", p]).0);
    let (_, out, _) = chasm(&["infer", p]);
    assert_eq!(out, "no un-annotated words\n");
    let _ = std::fs::remove_file(src);
}

/// A scratch program for the union tests: the shape union plus `extra`.
fn union_program(name: &str, extra: &str) -> std::path::PathBuf {
    let f = scratch(name);
    std::fs::write(
        &f,
        format!("union shape\n  | circle  r: f64\n  | rect    w: f64  h: f64\n  | empty\n{extra}"),
    )
    .unwrap();
    f
}

fn test_results(f: &std::path::Path) -> (bool, serde_json::Value) {
    let (ok, out, err) = chasm(&["test", "--json", f.to_str().unwrap()]);
    let j: serde_json::Value =
        serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}{err}"));
    (ok, j)
}

#[test]
fn union_tag_readers_and_traps() {
    let f = union_program(
        "union-tag.chasm",
        "test shape.tag : 2.0 3.0 shape.rect shape.tag -> 1\ntest shape.tag : shape.empty shape.tag -> 2\ntest shape.rect.h : 2.0 3.0 shape.rect shape.rect.h -> 3.0\n\
         : area ( shape -- f64 ) circle: [ :> r  r r f64.mul 3.14 f64.mul ] rect: [ f64.mul ] empty: [ 0.0 ] match ;\n\
         : n ( shape -- i32 ) circle: [ drop 1 ] rect: [ 2drop 2 ] empty: [ 3 ] match ;\n\
         : area2 ( shape -- f64 ) circle: [ :> r r r f64.mul 3.14 f64.mul ] else: [ drop 0.0 ] match ;\n\
         test area2 : 2.0 3.0 shape.rect area2 -> 0.0\ntest area2 : 1.0 shape.circle area2 -> 3.14\n\
         test area : 2.0 3.0 shape.rect area -> 6.0\ntest area : shape.empty area -> 0.0\ntest n : 1.0 shape.circle n -> 1\n",
    );
    let (ok, j) = test_results(&f);
    assert!(ok, "{j}");
    assert!(
        j["results"]["tests"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["status"] == "pass"),
        "{j}"
    );
    let (_, out, _) = chasm(&["words", "--json", f.to_str().unwrap()]);
    let w: serde_json::Value = serde_json::from_str(&out).unwrap();
    let tag = w["results"]["words"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["name"] == "shape.tag")
        .unwrap()
        .clone();
    assert_eq!(tag["generated"], true);
    let (_, out, _) = chasm(&["dead", f.to_str().unwrap()]);
    assert!(!out.contains("shape.tag"), "{out}");
    let g = union_program(
        "union-trap.chasm",
        "test shape.rect.h : 1.0 shape.circle shape.rect.h -> 0.0\n",
    );
    let (_, j) = test_results(&g);
    let t = &j["results"]["tests"][0];
    assert_eq!(t["status"], "fail", "{j}");
    assert_eq!(t["trap"]["message"], "shape.rect.h: not a rect", "{j}");
    let _ = std::fs::remove_file(f);
    let _ = std::fs::remove_file(g);
}

#[test]
fn generic_unions_run() {
    let src =
        "union option T | none | some  v: T\nunion list T | nil | cons  head: T  tail: list T\n\
               : length ( list T -- i32 ) nil: [ 0 ] cons: [ length 1 i32.add nip ] match ;\n\
               : three ( -- list i32 ) 1 2 3 list.nil list.cons list.cons list.cons ;\n\
               : sum ( list i32 -- i32 ) nil: [ 0 ] cons: [ sum i32.add ] match ;\n\
               : get ( option i32 -- i32 ) none: [ 0 ] some: [ ] match ;\n\
               test length : three length -> 3\ntest sum : three sum -> 6\n\
               test get : 5 option.some get -> 5\ntest get : option.none ( option i32 ) get -> 0\n\
               : main ( -- ) three sum i32.to-str println ;\n";
    let f = scratch("generic-unions.chasm");
    std::fs::write(&f, src).unwrap();
    let (ok, j) = test_results(&f);
    assert!(ok, "{j}");
    let tests = j["results"]["tests"].as_array().unwrap();
    assert_eq!(tests.len(), 4, "{j}");
    assert!(tests.iter().all(|t| t["status"] == "pass"), "{j}");
    let (ok, out, err) = chasm(&["run", f.to_str().unwrap()]);
    assert!(ok, "{err}");
    assert_eq!(out, "6\n");
    let _ = std::fs::remove_file(f);
}

#[test]
fn examples_and_benchmarks_are_formatted() {
    let mut files = Vec::new();
    for dir in ["examples", "bench"] {
        for entry in std::fs::read_dir(root().join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) == Some("chasm") {
                files.push(
                    path.strip_prefix(root())
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_string(),
                );
            }
        }
    }
    let mut args = vec!["fmt", "--check"];
    args.extend(files.iter().map(String::as_str));
    let (ok, out, err) = chasm(&args);
    assert!(
        ok,
        "run `chasm fmt examples/*.chasm bench/*.chasm`:\n{out}{err}"
    );
}
