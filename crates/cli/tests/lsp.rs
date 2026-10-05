//! Drive `wack lsp` over stdio with framed JSON-RPC messages.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

struct Lsp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Lsp {
    fn start() -> Lsp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_wack"))
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("run wack lsp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Lsp {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, msg: Value) {
        let body = msg.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn recv(&mut self) -> Value {
        let mut len = 0;
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).unwrap();
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(n) = line.strip_prefix("Content-Length: ") {
                len = n.parse().unwrap();
            }
        }
        let mut body = vec![0; len];
        self.stdout.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn initialize(&mut self) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"capabilities": {}}}));
        let r = self.recv();
        self.send(json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}));
        r
    }

    fn shutdown(mut self) {
        self.send(json!({"jsonrpc": "2.0", "id": 999, "method": "shutdown"}));
        let r = self.recv();
        assert!(r["result"].is_null(), "{r}");
        self.send(json!({"jsonrpc": "2.0", "method": "exit"}));
        let status = self.child.wait().unwrap();
        assert!(status.success());
    }
}

#[test]
fn initialize_and_shutdown() {
    let mut lsp = Lsp::start();
    let r = lsp.initialize();
    assert_eq!(r["result"]["capabilities"]["hoverProvider"], true, "{r}");
    assert_eq!(
        r["result"]["capabilities"]["definitionProvider"], true,
        "{r}"
    );
    lsp.shutdown();
}

impl Lsp {
    /// The next `textDocument/publishDiagnostics` notification.
    fn diagnostics(&mut self) -> Value {
        loop {
            let m = self.recv();
            if m["method"] == "textDocument/publishDiagnostics" {
                return m["params"].clone();
            }
        }
    }

    fn open(&mut self, uri: &str, text: &str) {
        self.send(
            json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
            "textDocument": {"uri": uri, "languageId": "wack", "version": 1, "text": text}}}),
        );
    }
}

#[test]
fn diagnostics_on_open_and_change() {
    let mut lsp = Lsp::start();
    lsp.initialize();
    let uri = "file:///tmp/x.wack";
    lsp.open(uri, ": f ( i32 -- i32 ) dup ;\n");
    let p = lsp.diagnostics();
    let ds = p["diagnostics"].as_array().unwrap();
    assert_eq!(ds.len(), 1, "{p}");
    assert_eq!(ds[0]["code"], "E_EFFECT_MISMATCH");
    assert_eq!(ds[0]["range"]["start"]["line"], 0);
    assert!(
        ds[0]["message"]
            .as_str()
            .unwrap()
            .contains("expected: ( i32 )"),
        "{p}"
    );
    lsp.send(
        json!({"jsonrpc": "2.0", "method": "textDocument/didChange", "params": {
        "textDocument": {"uri": uri, "version": 2},
        "contentChanges": [{"text": ": f ( i32 -- i32 ) dup i32.mul ;\n"}]}}),
    );
    let p = lsp.diagnostics();
    assert_eq!(p["diagnostics"].as_array().unwrap().len(), 0, "{p}");
    lsp.shutdown();
}

impl Lsp {
    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let m = self.recv();
            if m["id"] == id {
                return m["result"].clone();
            }
        }
    }

    fn at(&mut self, id: u64, method: &str, uri: &str, line: u32, character: u32) -> Value {
        self.request(
            id,
            method,
            json!({"textDocument": {"uri": uri}, "position": {"line": line, "character": character}}),
        )
    }
}

#[test]
fn hover_shows_effects() {
    let mut lsp = Lsp::start();
    lsp.initialize();
    let uri = "file:///tmp/h.wack";
    lsp.open(
        uri,
        ": sq ( i32 -- i32 ) dup i32.mul ;\ndeclare later ( str -- i32 )\n: twice ( i32 -- i32 ) sq sq ;\n",
    );
    lsp.diagnostics();
    let h = lsp.at(10, "textDocument/hover", uri, 2, 23);
    assert!(
        h["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("sq ( i32 -- i32 )"),
        "{h}"
    );
    let h = lsp.at(11, "textDocument/hover", uri, 0, 20);
    assert!(
        h["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("( a -- a a )"),
        "{h}"
    );
    let h = lsp.at(12, "textDocument/hover", uri, 1, 9);
    let v = h["contents"]["value"].as_str().unwrap();
    assert!(
        v.contains("( str -- i32 )") && v.contains("no body yet"),
        "{h}"
    );
    let h = lsp.at(13, "textDocument/hover", uri, 0, 40);
    assert!(h.is_null(), "{h}");
    lsp.shutdown();
}

#[test]
fn definition_of_user_words() {
    let mut lsp = Lsp::start();
    lsp.initialize();
    let uri = "file:///tmp/d.wack";
    lsp.open(
        uri,
        ": sq ( i32 -- i32 ) dup i32.mul ;\n: twice ( i32 -- i32 ) sq sq ;\n: say ( -- ) \"x\" println ;\n",
    );
    lsp.diagnostics();
    let d = lsp.at(20, "textDocument/definition", uri, 1, 23);
    assert_eq!(d["uri"], uri, "{d}");
    assert_eq!(d["range"]["start"]["line"], 0);
    assert_eq!(d["range"]["start"]["character"], 2);
    assert!(
        lsp.at(21, "textDocument/definition", uri, 0, 20).is_null(),
        "dup is a primitive"
    );
    assert!(
        lsp.at(22, "textDocument/definition", uri, 2, 17).is_null(),
        "println is in the prelude"
    );
    lsp.shutdown();
}

#[test]
fn hover_shows_inferred_and_generic_words() {
    let mut lsp = Lsp::start();
    lsp.initialize();
    let uri = "file:///tmp/g.wack";
    lsp.open(
        uri,
        ": sq dup i32.mul ;\n: twice ( T -- T T ) dup ;\n: a ( i32 -- i32 i32 ) twice ;\n",
    );
    lsp.diagnostics();
    let h = lsp.at(30, "textDocument/hover", uri, 0, 2);
    let v = h["contents"]["value"].as_str().unwrap();
    assert!(
        v.contains("sq ( i32 -- i32 )") && v.contains("inferred"),
        "{h}"
    );
    let h = lsp.at(31, "textDocument/hover", uri, 2, 24);
    let v = h["contents"]["value"].as_str().unwrap();
    assert!(
        v.contains("( T -- T T )") && v.contains("generic") && v.contains("instances: twice<i32>"),
        "{h}"
    );
    lsp.shutdown();
}
