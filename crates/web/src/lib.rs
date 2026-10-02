//! The Chasm compiler for the browser: the REPL session behind a small C
//! ABI, built as a `wasm32-unknown-unknown` cdylib. No wasm-bindgen; the
//! JavaScript side (`web/compiler.js`) passes UTF-8 text in through
//! `chasm_alloc` and reads results back as JSON and byte buffers.
//!
//! Buffers handed to JavaScript stay owned by Rust until the next call
//! that replaces them.

pub mod api {
    use std::cell::RefCell;

    use chasm_core::repl::{needs_more as core_needs_more, Step};
    use chasm_core::types::{names, Ty};
    use chasm_core::{layout, Diagnostic, Location, Session};
    use serde_json::{json, Value as J};

    /// One step's result: JSON plus the module and literal bytes.
    #[derive(Debug, Clone, Default)]
    pub struct StepJson {
        pub json: String,
        pub module: Vec<u8>,
        pub literals: Vec<u8>,
    }

    thread_local! {
        static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
        /// The last step's line result, committed by `line_done`.
        static PENDING: RefCell<Option<Vec<Ty>>> = const { RefCell::new(None) };
    }

    fn to_json(step: Step, stack: &[Ty], session: &Session) -> StepJson {
        let ok = step.ok();
        // Field layouts with the table slot of each accessor word: the
        // browser reads struct fields by calling `name.field` through the table.
        let structs: serde_json::Map<String, serde_json::Value> = session
            .structs()
            .iter()
            .map(|s| {
                let fields = s
                    .fields
                    .iter()
                    .map(|(f, t)| {
                        json!({
                            "field": f,
                            "type": t.to_string(),
                            "get": session.word_slot(&format!("{}.{f}", s.name)),
                        })
                    })
                    .collect::<Vec<_>>();
                (s.name.clone(), json!(fields))
            })
            .collect();
        let j = json!({
            "ok": ok,
            "diagnostics": step.diagnostics,
            "defined": step.defined.iter().map(|d| json!({
                "name": d.name, "effect": d.effect, "declared": d.declared,
            })).collect::<Vec<_>>(),
            "forgotten": step.forgotten,
            "forced": step.forced.iter().map(|f| json!({
                "name": f.name, "from": f.from, "to": f.to,
            })).collect::<Vec<_>>(),
            "rechecked": step.rechecked,
            "installs": step.installs.iter().map(|i| json!({
                "export": i.export, "slot": i.slot,
            })).collect::<Vec<_>>(),
            "table_size": step.table_size,
            "literal_addr": step.literal_addr,
            "line": step.line.as_ref().map(|l| json!({
                "slot": l.slot, "export": l.export, "stack_after": names(&l.stack_after),
            })),
            "tests": step.tests.iter().map(|t| json!({
                "slot": t.slot,
                "word": t.word,
                "expected": t.expected,
                // Exact text: JSON numbers cannot carry every i64.
                "expected_text": t.expected.iter().map(|v| v.to_string()).collect::<Vec<_>>(),
                "types": t.types,
                "location": t.location,
            })).collect::<Vec<_>>(),
            "stack": names(stack),
            "refs_size": step.refs_size,
            "structs": structs,
        });
        PENDING.with(|p| *p.borrow_mut() = step.line.map(|l| l.stack_after));
        StepJson {
            json: j.to_string(),
            module: step.module.unwrap_or_default(),
            literals: step.literal_bytes,
        }
    }

    /// Start a session with a shared memory; returns the prelude step.
    pub fn new_session(prelude: bool, heap_ptr: u32) -> StepJson {
        let (session, step) = Session::new(prelude, true, heap_ptr);
        let out = to_json(step, &session.stack, &session);
        SESSION.with(|s| *s.borrow_mut() = Some(session));
        out
    }

    pub fn step(text: &str, heap_ptr: u32) -> StepJson {
        SESSION.with(|s| match s.borrow_mut().as_mut() {
            Some(session) => {
                let stack = session.stack.clone();
                let step = session.step(text, heap_ptr);
                to_json(step, &stack, session)
            }
            None => {
                let d = Diagnostic::error(
                    chasm_core::diag::codes::E_INTERNAL,
                    "no session: call chasm_new first",
                    Location::default(),
                );
                StepJson {
                    json: json!({ "ok": false, "diagnostics": [d] }).to_string(),
                    ..StepJson::default()
                }
            }
        })
    }

    /// The host ran the last step's line: commit its stack if it succeeded.
    pub fn line_done(ok: bool) {
        let pending = PENDING.with(|p| p.borrow_mut().take());
        if let (true, Some(stack)) = (ok, pending) {
            SESSION.with(|s| {
                if let Some(session) = s.borrow_mut().as_mut() {
                    session.stack = stack;
                }
            });
        }
    }

    pub fn needs_more(text: &str) -> bool {
        core_needs_more(text)
    }

    /// The memory layout as a JSON object, so JavaScript never hard-codes it.
    pub fn layout_json() -> String {
        let map: serde_json::Map<String, J> = layout::constants()
            .into_iter()
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect();
        J::Object(map).to_string()
    }
}

use std::cell::RefCell;

thread_local! {
    static RESULT: RefCell<api::StepJson> = RefCell::new(api::StepJson::default());
    static LAYOUT: String = api::layout_json();
}

fn set_result(r: api::StepJson) {
    RESULT.with(|x| *x.borrow_mut() = r);
}

/// # Safety
/// The returned buffer of `len` bytes must be released with `chasm_free`.
#[no_mangle]
pub extern "C" fn chasm_alloc(len: u32) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len as usize);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` and `len` must come from one `chasm_alloc` call.
#[no_mangle]
pub unsafe extern "C" fn chasm_free(ptr: *mut u8, len: u32) {
    drop(Vec::from_raw_parts(ptr, 0, len as usize));
}

/// # Safety
/// `ptr..ptr+len` must be readable.
unsafe fn text<'a>(ptr: *const u8, len: u32) -> std::borrow::Cow<'a, str> {
    String::from_utf8_lossy(std::slice::from_raw_parts(ptr, len as usize))
}

#[no_mangle]
pub extern "C" fn chasm_new(prelude: i32, heap_ptr: u32) {
    set_result(api::new_session(prelude != 0, heap_ptr));
}

/// # Safety
/// `ptr..ptr+len` must be readable UTF-8 text.
#[no_mangle]
pub unsafe extern "C" fn chasm_step(ptr: *const u8, len: u32, heap_ptr: u32) {
    set_result(api::step(&text(ptr, len), heap_ptr));
}

#[no_mangle]
pub extern "C" fn chasm_line_done(ok: i32) {
    api::line_done(ok != 0);
}

/// # Safety
/// `ptr..ptr+len` must be readable UTF-8 text.
#[no_mangle]
pub unsafe extern "C" fn chasm_needs_more(ptr: *const u8, len: u32) -> i32 {
    api::needs_more(&text(ptr, len)) as i32
}

macro_rules! buffer {
    ($ptr:ident, $len:ident, $field:ident) => {
        #[no_mangle]
        pub extern "C" fn $ptr() -> *const u8 {
            RESULT.with(|r| r.borrow().$field.as_ptr())
        }
        #[no_mangle]
        pub extern "C" fn $len() -> u32 {
            RESULT.with(|r| r.borrow().$field.len() as u32)
        }
    };
}

buffer!(chasm_result_ptr, chasm_result_len, json);
buffer!(chasm_module_ptr, chasm_module_len, module);
buffer!(chasm_literals_ptr, chasm_literals_len, literals);

#[no_mangle]
pub extern "C" fn chasm_layout_ptr() -> *const u8 {
    LAYOUT.with(|l| l.as_ptr())
}

#[no_mangle]
pub extern "C" fn chasm_layout_len() -> u32 {
    LAYOUT.with(|l| l.len() as u32)
}

#[cfg(test)]
mod tests {
    use super::api::*;
    use serde_json::Value as J;

    fn j(s: &StepJson) -> J {
        serde_json::from_str(&s.json).unwrap()
    }

    #[test]
    fn session_through_the_api() {
        let hp = 0x20_0000;
        let s = new_session(true, 0x10_0000);
        assert!(j(&s)["table_size"].as_u64().unwrap() > 10);
        assert!(!s.module.is_empty());
        let s = step(": sq ( i32 -- i32 ) dup i32.mul ;", hp);
        assert_eq!(j(&s)["installs"].as_array().unwrap().len(), 1);
        let s = step("3 sq", hp);
        assert_eq!(j(&s)["line"]["stack_after"], serde_json::json!(["i32"]));
        line_done(true);
        let s = step("sq", hp);
        assert_eq!(j(&s)["stack"], serde_json::json!(["i32"]));
        assert!(needs_more(": f ( -- )"));
        assert!(layout_json().contains("\"DATA_STACK_PTR\":276"));
        assert!(layout_json().contains("\"REFS_TABLE\":1"));
        line_done(true);
        let s = step("drop struct point  x: i32  y: f64", hp);
        assert!(j(&s)["ok"] == false, "a line cannot follow a definition");
        let s = step("struct point  x: i32  y: f64", hp);
        let fields = &j(&s)["structs"]["point"];
        assert_eq!(fields[0]["field"], "x");
        assert_eq!(fields[1]["field"], "y");
        assert_eq!(fields[0]["type"], "i32");
        assert_eq!(fields[1]["type"], "f64");
        assert!(fields[0]["get"].is_u64() && fields[1]["get"].is_u64());
        let s = step("drop 7 2.5 point.new", hp);
        assert_eq!(j(&s)["refs_size"], 1);
        assert_eq!(j(&s)["line"]["stack_after"], serde_json::json!(["point"]));
    }
}
