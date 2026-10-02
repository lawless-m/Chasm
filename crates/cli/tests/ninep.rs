//! `chasm run` with a 9p server mounted at /mnt/p.

#[path = "../../runtime/tests/common/ninep_server.rs"]
mod server;

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
fn run_with_a_9p_mount() {
    let addr = server::start();
    std::fs::create_dir_all(root().join("tmp")).unwrap();
    let prog = root().join(format!("tmp/ninep-{}.chasm", std::process::id()));
    std::fs::write(
        &prog,
        ": main ( -- ) \"/mnt/p\" ls drop \"/mnt/p/hello.txt\" read-file drop print ;\n",
    )
    .unwrap();
    let p = prog.to_str().unwrap();
    let mount = format!("p=9p://{addr}");
    let (ok, out, err) = chasm(&["run", p, "--mount", &mount]);
    assert!(ok, "{out}{err}");
    assert!(out.contains("hello.txt\n"), "{out}");
    assert!(out.contains("sub/\n"), "{out}");
    assert!(out.contains("hello\n"), "{out}");
    let (ok, _, err) = chasm(&["run", p, "--mount", "p=9p://127.0.0.1"]);
    assert!(!ok);
    assert!(err.contains("E_USAGE"), "{err}");
    let _ = std::fs::remove_file(prog);
}
