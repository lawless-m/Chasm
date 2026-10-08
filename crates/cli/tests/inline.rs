//! The compiler's inliner seen through the `wack` binary: traps keep the inlined
//! word's name, results are unchanged, and the graph commands still see the
//! inlined callee.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("wack-inline-{}-{name}", std::process::id()))
}

fn wack(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_wack"))
        .args(args)
        .current_dir(root())
        .output()
        .expect("run wack");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn write(name: &str, src: &str) -> String {
    let p = scratch(name);
    std::fs::write(&p, src).unwrap();
    p.to_str().unwrap().to_string()
}

#[test]
fn trap_in_an_inlined_word_names_it() {
    let f = write(
        "trap.wack",
        ": inner ( i32 -- i32 ) dup 0 i32.lt_s [ \"negative\" trap ] when ;\n: main ( -- ) -1 inner drop ;\n",
    );
    let (ok, _, err) = wack(&["run", &f]);
    assert!(!ok);
    assert!(err.contains("trap in `inner`: negative"), "{err}");
    let (ok, json, _) = wack(&["run", "--json", &f]);
    assert!(!ok);
    let j: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(j["results"]["trap"]["word"], "inner", "{json}");

    // A bounds trap inside a small, inlined word, and the same word made too
    // big to inline: the same message either way.
    let small = write(
        "small.wack",
        ": get ( array i32 i32 -- i32 ) array.at ;\n: main ( -- ) 3 array.new ( array i32 ) :> a  a 5 get drop ;\n",
    );
    let pad = "  0 i32.add\n".repeat(200);
    let big = write(
        "big.wack",
        &format!(": get ( array i32 i32 -- i32 )\n  array.at\n{pad}  ;\n: main ( -- ) 3 array.new ( array i32 ) :> a  a 5 get drop ;\n"),
    );
    let (ok1, _, e1) = wack(&["run", &small]);
    let (ok2, _, e2) = wack(&["run", &big]);
    assert!(!ok1 && !ok2);
    assert!(e1.contains("trap in `get`"), "{e1}");
    assert_eq!(
        e1.lines().find(|l| l.contains("trap in")),
        e2.lines().find(|l| l.contains("trap in")),
        "{e1}\n{e2}"
    );
}

#[test]
fn inlined_results_match() {
    let f = write(
        "results.wack",
        ": wide ( i32 -- i64 ) i64 ;\n\
         : three ( -- i32 i32 i32 ) 1 2 3 ;\n\
         : mix ( i32 i32 -- i32 ) :> b :> a  a b i32.mul a i32.add ;\n\
         : sum8 ( -- i32 ) 0 :> s!  8 [ s i32.add s! ] times  s ;\n\
         : big ( -- i64 ) three i32.add i32.add  2 3 mix i32.add  sum8 i32.add  wide  7 wide i64.add ;\n\
         : twice ( -- i32 ) 2 3 mix  4 5 mix i32.add ;\n\
         test wide : 5 wide -> 5 i64\n\
         test three : three -> 1 2 3\n\
         test mix : 2 3 mix -> 8\n\
         test sum8 : sum8 -> 28\n\
         test big : big -> 49 i64\n\
         test twice : twice -> 32\n",
    );
    let (ok, json, err) = wack(&["test", "--json", &f]);
    assert!(ok, "{json}{err}");
    let j: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(j["results"]["summary"]["pass"], 6, "{json}");
    assert_eq!(j["results"]["summary"]["fail"], 0, "{json}");
}

#[test]
fn graph_commands_see_inlined_callees() {
    let f = write(
        "graph.wack",
        ": helper ( i32 -- i32 ) 1 i32.add ;\n: unused ( -- i32 ) 7 ;\n: main ( -- ) 1 helper drop ;\n",
    );
    let words = |args: &[&str]| -> Vec<String> {
        let (ok, json, err) = wack(args);
        assert!(ok, "{json}{err}");
        let j: serde_json::Value = serde_json::from_str(&json).unwrap();
        let key = if j["results"]["dead"].is_array() {
            "dead"
        } else {
            "words"
        };
        j["results"][key]
            .as_array()
            .unwrap()
            .iter()
            // deps and dead list objects with a word; used-by lists names.
            .map(|w| w.as_str().or(w["word"].as_str()).unwrap().to_string())
            .collect()
    };
    assert!(words(&["deps", "--json", "main", &f]).contains(&"helper".to_string()));
    assert!(words(&["used-by", "--json", "helper", &f]).contains(&"main".to_string()));
    assert_eq!(words(&["dead", "--json", &f]), ["unused"]);
}
