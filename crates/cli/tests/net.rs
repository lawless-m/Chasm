//! `chasm run` making HTTP requests against a local server.

#[path = "../../runtime/tests/common/http_server.rs"]
mod http_server;

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

const PROGRAM: &str = r#"
: drain ( i32 -- str i32 )
  :> h
  4096 bytes.new :> buf
  h buf host.read :> n
  h host.close drop
  n 0 i32.lt_s [ "" n ] [ buf 0 n bytes.slice bytes.to-str 0 ] if ;

: post ( str str -- str i32 )
  :> req :> path
  path 3 host.open :> h
  h 0 i32.lt_s
  [ "" h ]
  [ h req host.write drop  h drain ]
  if ;

: main ( -- )
  "/net/http/ADDR/hello" read-file :> status :> body
  body print  status i32.to-str println
  "/net/http/ADDR/post" "X-Chasm: 7\n\nhello" post drop println
  "/net/http/ADDR/missing" read-file i32.to-str println drop ;
"#;

#[test]
fn run_gets_and_posts() {
    let addr = http_server::start();
    std::fs::create_dir_all(root().join("tmp")).unwrap();
    let prog = root().join(format!("tmp/net-{}.chasm", std::process::id()));
    std::fs::write(&prog, PROGRAM.replace("ADDR", &addr.to_string())).unwrap();
    let p = prog.to_str().unwrap();
    let (ok, out, err) = chasm(&["run", p]);
    assert!(ok, "{out}{err}");
    assert!(out.contains("hi\n0\n"), "{out}");
    assert!(out.contains("got:hello:7"), "{out}");
    assert!(out.contains("-1\n"), "{out}");
    let (_, out, _) = chasm(&["run", "--no-net", p]);
    assert!(out.starts_with("-3\n"), "{out}");
    let _ = std::fs::remove_file(prog);
}
