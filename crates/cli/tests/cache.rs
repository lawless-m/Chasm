//! The compile cache through the `wack` binary, each test on its own cache
//! directory (`XDG_CACHE_HOME`), so tests running in parallel never share one.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh directory for one test, removed when the guard drops.
struct Tmp(PathBuf);

impl Tmp {
    fn new(name: &str) -> Tmp {
        let d = std::env::temp_dir().join(format!("wack-cli-cache-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Tmp(d)
    }

    /// The compile cache's entries.
    fn entries(&self) -> Vec<PathBuf> {
        match std::fs::read_dir(self.0.join("wack/compile")) {
            Ok(rd) => {
                let mut v: Vec<PathBuf> = rd
                    .map(|e| e.unwrap().path())
                    .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'))
                    .collect();
                v.sort();
                v
            }
            Err(_) => Vec::new(),
        }
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wack(cache: &Path, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_wack"))
        .args(args)
        .current_dir(root())
        .env("XDG_CACHE_HOME", cache)
        .stdin(Stdio::null())
        .output()
        .expect("run wack");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn hit_matches_miss() {
    let t = Tmp::new("hit");
    let (ok1, out1, _) = wack(&t.0, &["run", "examples/hello.wack"]);
    assert!(ok1);
    assert_eq!(t.entries().len(), 1);
    let (ok2, out2, _) = wack(&t.0, &["run", "examples/hello.wack"]);
    assert!(ok2);
    assert_eq!(out1, out2);
    assert_eq!(t.entries().len(), 1);
}

#[test]
fn changed_source_misses() {
    let t = Tmp::new("changed");
    let file = t.0.join("hello.wack");
    let text = std::fs::read_to_string(root().join("examples/hello.wack")).unwrap();
    std::fs::write(&file, &text).unwrap();
    let f = file.to_str().unwrap();
    let (ok1, out1, _) = wack(&t.0, &["run", f]);
    assert!(ok1);
    assert_eq!(out1, "Hello from Whackford\n");
    std::fs::write(&file, text.replace("Hello from Whackford", "Hello again")).unwrap();
    let (ok2, out2, _) = wack(&t.0, &["run", f]);
    assert!(ok2);
    assert_eq!(out2, "Hello again\n");
    assert_eq!(t.entries().len(), 2);
}

#[test]
fn run_and_test_are_separate_entries() {
    let t = Tmp::new("separate");
    let (ok1, out1, _) = wack(&t.0, &["test", "examples/bytes.wack"]);
    assert!(ok1);
    let (ok2, out2, _) = wack(&t.0, &["test", "examples/bytes.wack"]);
    assert!(ok2);
    assert_eq!(out1, out2);
    assert!(out2.trim_end().ends_with("0 failed, 0 pending"), "{out2}");
    assert_eq!(t.entries().len(), 1);
    let (ok3, _, _) = wack(
        &t.0,
        &["run", "examples/bytes.wack", "--mount", "ex=examples"],
    );
    assert!(ok3);
    assert_eq!(t.entries().len(), 2);
}

#[test]
fn corrupt_entry_falls_back() {
    let t = Tmp::new("corrupt");
    assert!(wack(&t.0, &["run", "examples/hello.wack"]).0);
    let e = t.entries();
    assert_eq!(e.len(), 1);
    let good = std::fs::read(&e[0]).unwrap();
    let garbage: &[u8] = b"not an entry\n\x00\x01";
    std::fs::write(&e[0], garbage).unwrap();
    let (ok, out, _) = wack(&t.0, &["run", "examples/hello.wack"]);
    assert!(ok);
    assert_eq!(out, "Hello from Whackford\n");
    assert_ne!(std::fs::read(&e[0]).unwrap(), garbage);
    std::fs::write(&e[0], &good[..40]).unwrap();
    let (ok, out, _) = wack(&t.0, &["run", "examples/hello.wack"]);
    assert!(ok);
    assert_eq!(out, "Hello from Whackford\n");
    assert_eq!(std::fs::read(&e[0]).unwrap(), good);
}

#[test]
fn unwritable_cache_dir_falls_back() {
    let t = Tmp::new("unwritable");
    let file = t.0.join("not-a-dir");
    std::fs::write(&file, "x").unwrap();
    let (ok, out, err) = wack(&file, &["run", "examples/hello.wack"]);
    assert!(ok, "{err}");
    assert_eq!(out, "Hello from Whackford\n");
    assert!(!err.contains("panicked"), "{err}");
}
