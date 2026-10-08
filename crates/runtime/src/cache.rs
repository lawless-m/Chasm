//! The compile cache: `wack run` and `wack test` keep the Whackford compile's
//! output on disk, beside wasmtime's module cache, so an unchanged program
//! skips the front end as wasmtime skips Cranelift.
//!
//! An entry is one file, `<cache root>/compile/<key>`: a JSON header line
//! (format, key, `has_main`, diagnostics, tests with their result types, the
//! wasm's length) and then the wasm. The key hashes everything that decides
//! the output: the format, the compiler's version and executable, the
//! prelude, the options and every source's name and text in order. A missing,
//! corrupt or mismatched entry, or a cache that cannot be written, costs only
//! the compile.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use wack_core::types::Ty;
use wack_core::{Compilation, Diagnostic, Options, Source, TestInfo};

const FORMAT: &str = "wack-compile-1";

/// What the hosts use from a compile.
pub struct Cached {
    pub diagnostics: Vec<Diagnostic>,
    pub has_main: bool,
    pub tests: Vec<TestInfo>,
    pub wasm: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct Header {
    format: String,
    key: String,
    has_main: bool,
    diagnostics: Vec<Diagnostic>,
    tests: Vec<(TestInfo, Vec<Ty>)>,
    wasm_len: usize,
}

/// `$XDG_CACHE_HOME/wack`, else `$HOME/.cache/wack`.
pub(crate) fn cache_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        Some(x) => PathBuf::from(x),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".cache"),
    };
    Some(base.join("wack"))
}

fn entry_dir(root: &Path) -> PathBuf {
    root.join("compile")
}

/// The entry's name: the SHA-256 of every input, each preceded by its length
/// so that boundaries cannot shift. The executable's length and modification
/// time tell compiler builds apart (the package version does not change
/// between them); without them there is no key and no caching.
pub fn key(sources: &[Source], opts: &Options) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let meta = std::fs::metadata(exe).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    let mut h = Sha256::new();
    let mut item = |b: &[u8]| {
        h.update((b.len() as u64).to_le_bytes());
        h.update(b);
    };
    item(FORMAT.as_bytes());
    item(env!("CARGO_PKG_VERSION").as_bytes());
    item(&meta.len().to_le_bytes());
    item(&mtime.as_nanos().to_le_bytes());
    item(wack_core::program::PRELUDE.as_bytes());
    item(&[
        opts.prelude as u8,
        opts.test_exports as u8,
        opts.export as u8,
        opts.wasi as u8,
    ]);
    for s in sources {
        item(s.name.as_bytes());
        item(s.text.as_bytes());
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn load_from(dir: &Path, key: &str) -> Option<Cached> {
    let f = std::fs::File::open(entry_dir(dir).join(key)).ok()?;
    let mut r = std::io::BufReader::new(f);
    let mut line = String::new();
    r.read_line(&mut line).ok()?;
    let h: Header = serde_json::from_str(line.strip_suffix('\n')?).ok()?;
    if h.format != FORMAT || h.key != key {
        return None;
    }
    let mut wasm = Vec::new();
    r.read_to_end(&mut wasm).ok()?;
    if wasm.len() != h.wasm_len {
        return None;
    }
    let tests = h
        .tests
        .into_iter()
        .map(|(mut t, types)| {
            t.result_types = types;
            t
        })
        .collect();
    Some(Cached {
        diagnostics: h.diagnostics,
        has_main: h.has_main,
        tests,
        wasm,
    })
}

fn store_in(dir: &Path, key: &str, c: &Compilation, with_tests: bool) {
    let Some(wasm) = &c.wasm else { return };
    let tests = if with_tests {
        c.tests
            .iter()
            .map(|t| (t.clone(), t.result_types.clone()))
            .collect()
    } else {
        Vec::new()
    };
    let h = Header {
        format: FORMAT.into(),
        key: key.into(),
        has_main: c.has_main,
        diagnostics: c.diagnostics.clone(),
        tests,
        wasm_len: wasm.len(),
    };
    let Ok(mut bytes) = serde_json::to_vec(&h) else {
        return;
    };
    bytes.push(b'\n');
    bytes.extend_from_slice(wasm);
    let d = entry_dir(dir);
    if std::fs::create_dir_all(&d).is_err() {
        return;
    }
    // A temporary name and a rename: a concurrent reader sees the old entry,
    // none, or the whole new one, never part of it.
    let tmp = d.join(format!(".{key}.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, &bytes).is_err() || std::fs::rename(&tmp, d.join(key)).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Compile through the cache: a hit is returned as stored; a miss compiles,
/// stores what it produced and returns it. `Err` carries the diagnostics of a
/// compile that produced no module.
pub fn compile(sources: &[Source], opts: &Options, cache: bool) -> Result<Cached, Vec<Diagnostic>> {
    let place = if cache {
        key(sources, opts).zip(cache_dir())
    } else {
        None
    };
    if let Some((k, dir)) = &place {
        if let Some(hit) = load_from(dir, k) {
            return Ok(hit);
        }
    }
    let c = wack_core::compile(sources, opts);
    if let Some((k, dir)) = &place {
        store_in(dir, k, &c, opts.test_exports);
    }
    match c.wasm {
        Some(wasm) => Ok(Cached {
            diagnostics: c.diagnostics,
            has_main: c.has_main,
            tests: c.tests,
            wasm,
        }),
        None => Err(c.diagnostics),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wack-cache-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn src() -> Vec<Source> {
        vec![Source::new(
            "t.wack",
            ": sq ( i32 -- i32 )  dup i32.mul ;\ntest sq : 3 sq -> 9\n: main ( -- )  2 sq drop ;\n",
        )]
    }

    fn opts(test_exports: bool) -> Options {
        Options {
            prelude: true,
            test_exports,
            export: !test_exports,
            wasi: false,
        }
    }

    #[test]
    fn key_is_stable_and_covers_every_input() {
        let s = src();
        let o = opts(false);
        let k = key(&s, &o).unwrap();
        assert_eq!(k, key(&s, &o).unwrap());
        assert_eq!(k.len(), 64);
        let mut text = src();
        text[0].text.push(' ');
        assert_ne!(k, key(&text, &o).unwrap());
        let mut name = src();
        name[0].name = "u.wack".into();
        assert_ne!(k, key(&name, &o).unwrap());
        let two = vec![Source::new("a", "x"), Source::new("b", "y")];
        let swapped = vec![Source::new("b", "y"), Source::new("a", "x")];
        assert_ne!(key(&two, &o).unwrap(), key(&swapped, &o).unwrap());
        let shifted = vec![Source::new("a", "xb"), Source::new("", "y")];
        assert_ne!(key(&two, &o).unwrap(), key(&shifted, &o).unwrap());
        let flips: [fn(&mut Options); 4] = [
            |o| o.prelude = !o.prelude,
            |o| o.test_exports = !o.test_exports,
            |o| o.export = !o.export,
            |o| o.wasi = !o.wasi,
        ];
        for flip in flips {
            let mut p = opts(false);
            flip(&mut p);
            assert_ne!(k, key(&s, &p).unwrap());
        }
    }

    #[test]
    fn round_trip_keeps_tests_and_their_types() {
        let d = tmpdir("round");
        let s = src();
        let o = opts(true);
        let c = wack_core::compile(&s, &o);
        assert!(c.wasm.is_some() && !c.tests.is_empty());
        store_in(&d, "k1", &c, true);
        let hit = load_from(&d, "k1").unwrap();
        assert_eq!(Some(&hit.wasm), c.wasm.as_ref());
        assert_eq!(hit.has_main, c.has_main);
        assert_eq!(hit.diagnostics, c.diagnostics);
        assert_eq!(hit.tests.len(), c.tests.len());
        for (a, b) in hit.tests.iter().zip(&c.tests) {
            assert_eq!(a.result_types, b.result_types);
            assert_eq!(
                serde_json::to_string(a).unwrap(),
                serde_json::to_string(b).unwrap()
            );
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn garbled_entry_misses() {
        let d = tmpdir("garbled");
        let c = wack_core::compile(&src(), &opts(false));
        store_in(&d, "k2", &c, false);
        assert!(load_from(&d, "k2").is_some());
        std::fs::write(entry_dir(&d).join("k2"), "not an entry\n\0\x01").unwrap();
        assert!(load_from(&d, "k2").is_none());
        let full = {
            store_in(&d, "k2", &c, false);
            std::fs::read(entry_dir(&d).join("k2")).unwrap()
        };
        std::fs::write(entry_dir(&d).join("k2"), &full[..full.len() - 1]).unwrap();
        assert!(load_from(&d, "k2").is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn wrong_key_misses() {
        let d = tmpdir("wrongkey");
        let c = wack_core::compile(&src(), &opts(false));
        store_in(&d, "k3", &c, false);
        std::fs::rename(entry_dir(&d).join("k3"), entry_dir(&d).join("k4")).unwrap();
        assert!(load_from(&d, "k4").is_none());
        assert!(load_from(&d, "k3").is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn unwritable_dir_is_ignored() {
        let d = tmpdir("unwritable");
        let file = d.join("plain-file");
        std::fs::write(&file, "x").unwrap();
        let c = wack_core::compile(&src(), &opts(false));
        store_in(&file, "k5", &c, false);
        assert!(load_from(&file, "k5").is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn run_entry_carries_no_tests() {
        let d = tmpdir("runmode");
        let c = wack_core::compile(&src(), &opts(true));
        assert!(!c.tests.is_empty());
        store_in(&d, "k6", &c, false);
        assert!(load_from(&d, "k6").unwrap().tests.is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
