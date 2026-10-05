//! `wack prims` lists every primitive with its effect.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
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

#[test]
fn every_primitive_has_an_effect() {
    let (ok, out, err) = wack(&["prims", "--json"]);
    assert!(ok, "prims --json failed:\n{out}{err}");
    let report: serde_json::Value = serde_json::from_str(&out).expect("a JSON report");
    assert_eq!(report["ok"], true);
    assert_eq!(report["command"], "prims");
    let prims = report["results"]["primitives"].as_array().unwrap();
    let effect = |name: &str| {
        prims
            .iter()
            .find(|p| p["name"] == name)
            .unwrap_or_else(|| panic!("no primitive `{name}`"))["effect"]
            .as_str()
            .unwrap()
    };
    for p in prims {
        assert!(
            !p["effect"].as_str().unwrap().is_empty(),
            "{} has no effect",
            p["name"]
        );
    }
    assert_eq!(effect("dup"), "( a -- a a )");
    assert_eq!(effect("i32.add"), "( i32 i32 -- i32 )");
    effect("match");
}

#[test]
fn plain_text_lists_one_primitive_per_line() {
    let (ok, out, err) = wack(&["prims"]);
    assert!(ok, "prims failed:\n{out}{err}");
    assert!(out.lines().any(|l| l.starts_with("dup ( a -- a a )")));
}
