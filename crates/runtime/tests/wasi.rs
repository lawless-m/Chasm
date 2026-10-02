//! A `--wasi` build run under wasmtime-wasi's preview1, offline.

use std::path::PathBuf;

use chasm_core::{compile, Options, Source};
use wasmtime::{Linker, Module, Store};
use wasmtime_wasi::p1::{add_to_linker_sync, WasiP1Ctx};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{FsPerms, WasiCtxBuilder};

fn build(src: &str) -> Vec<u8> {
    let c = compile(
        &[Source::new("t.chasm", src)],
        &Options {
            prelude: true,
            test_exports: false,
            export: true,
            wasi: true,
        },
    );
    assert!(c.ok(), "{:?}", c.diagnostics);
    c.wasm.unwrap()
}

/// A scratch directory under the repository's git-ignored `tmp/`.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp")
        .join(format!("wasi-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `_start` with `dir` preopened as `/`; stdout, and whether it succeeded.
fn run(wasm: &[u8], dir: &PathBuf) -> (String, bool) {
    let engine = chasm_runtime::native::engine().unwrap();
    let stdout = MemoryOutputPipe::new(1 << 20);
    let ctx = WasiCtxBuilder::new()
        .stdout(stdout.clone())
        .preopened_dir(dir, "/", FsPerms::ReadWrite)
        .unwrap()
        .build_p1();
    let mut store: Store<WasiP1Ctx> = Store::new(&engine, ctx);
    let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
    add_to_linker_sync(&mut linker, |t| t).unwrap();
    let module = Module::new(&engine, wasm).unwrap();
    let instance = linker.instantiate(&mut store, &module).unwrap();
    let start = instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .unwrap();
    let ok = start.call(&mut store, ()).is_ok();
    drop(store);
    (String::from_utf8(stdout.contents().to_vec()).unwrap(), ok)
}

const PROGRAM: &str = r#"
: main ( -- )
  "hello from wasi" println
  "/file/in.txt" read-file drop print
  "/file/out.txt" 1 host.open :> h
  h "written" str.addr 7 host.write drop
  h host.close drop
  now 0 i64 i64.gt_s [ "time ok" ] [ "time bad" ] if println
  "/dev/nope" 0 host.open i32.to-str println ;
"#;

#[test]
fn console_files_time_and_errors() {
    let dir = scratch("io");
    std::fs::write(dir.join("in.txt"), "from file\n").unwrap();
    let (out, ok) = run(&build(PROGRAM), &dir);
    assert!(ok, "{out}");
    assert_eq!(out, "hello from wasi\nfrom file\ntime ok\n-1\n");
    assert_eq!(
        std::fs::read_to_string(dir.join("out.txt")).unwrap(),
        "written"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_trap_fails_start() {
    let dir = scratch("trap");
    let (_, ok) = run(&build(": main ( -- ) \"boom\" trap ;"), &dir);
    assert!(!ok);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_wasi_example_runs() {
    let src = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/wasi.chasm"),
    )
    .unwrap();
    let dir = scratch("example");
    std::fs::write(dir.join("README.md"), "# A preopened readme\n").unwrap();
    let (out, ok) = run(&build(&src), &dir);
    assert!(ok, "{out}");
    assert!(out.contains("hello from wasi\n"), "{out}");
    assert!(out.contains("ok: # A preopened readm"), "{out}");
    assert!(out.contains("time ok\n"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}
