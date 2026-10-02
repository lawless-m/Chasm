use chasm_core::layout::LITERALS_BASE;
use chasm_core::types::Ty;
use chasm_core::{validate, Session, Step, Value};

fn ok(s: &mut Session, text: &str) -> Step {
    let step = s.step(text, 0x20_0000);
    for d in &step.diagnostics {
        eprintln!("{}", d.render());
    }
    assert!(step.ok(), "step `{text}` failed");
    if let Some(m) = &step.module {
        validate(m).unwrap();
    }
    step
}

fn code(s: &mut Session, text: &str) -> String {
    let step = s.step(text, 0x20_0000);
    assert!(!step.ok(), "step `{text}` should fail");
    assert!(step.line.is_none());
    step.diagnostics[0].code.clone()
}

fn session() -> Session {
    let (s, step) = Session::new(true, false, LITERALS_BASE);
    assert!(step.ok());
    validate(step.module.as_ref().unwrap()).unwrap();
    assert_eq!(step.installs.len() as u32, step.table_size);
    assert!(step.table_size > 10);
    assert_eq!(step.literal_addr, LITERALS_BASE);
    assert!(step.literal_bytes.windows(9).any(|w| w == b"/dev/cons"));
    s
}

#[test]
fn prelude_step() {
    session();
}

#[test]
fn define_and_run_lines() {
    let mut s = session();
    let step = ok(&mut s, ": sq ( i32 -- i32 ) dup i32.mul ;");
    assert_eq!(step.defined.len(), 1);
    assert_eq!(step.defined[0].name, "sq");
    assert_eq!(step.defined[0].effect, "( i32 -- i32 )");
    assert!(!step.defined[0].declared);
    assert_eq!(step.installs.len(), 1);
    assert_eq!(step.installs[0].slot, step.table_size - 1);
    assert!(step.line.is_none());

    let step = ok(&mut s, "3 sq");
    assert_eq!(step.line.unwrap().stack_after, vec![Ty::I32]);
    s.stack = vec![Ty::I32];
    let step = ok(&mut s, "sq");
    assert_eq!(step.line.unwrap().stack_after, vec![Ty::I32]);
    s.stack = vec![];
    assert_eq!(code(&mut s, "sq"), "E_STACK_UNDERFLOW");
}

#[test]
fn redefinition_keeps_the_slot() {
    let mut s = session();
    let sq = ok(&mut s, ": sq ( i32 -- i32 ) dup i32.mul ;").installs[0].slot;
    ok(&mut s, ": twice ( i32 -- i32 ) sq sq ;");
    let step = ok(&mut s, ": sq ( i32 -- i32 ) 2 i32.mul ;");
    assert_eq!(step.installs.len(), 1);
    assert_eq!(step.installs[0].slot, sq);
    let step = s.step(": sq ( i32 i32 -- i32 ) i32.add ;", 0x20_0000);
    assert_eq!(step.diagnostics[0].code, "E_REDEFINE_EFFECT");
    assert_eq!(
        step.diagnostics[0].dependants,
        Some(vec!["twice".to_string()])
    );
}

#[test]
fn declare_installs_a_stub() {
    let mut s = session();
    let step = ok(&mut s, "declare g ( -- i32 )");
    assert_eq!(step.defined.len(), 1);
    assert!(step.defined[0].declared);
    assert_eq!(step.installs.len(), 1);
}

#[test]
fn literals_are_placed_at_the_heap_pointer() {
    let mut s = session();
    let step = s.step("\"hi\"", 0x30_0000);
    assert!(step.ok());
    assert_eq!(step.literal_addr, 0x30_0000);
    assert!(step.literal_bytes.windows(2).any(|w| w == b"hi"));
    assert_eq!(step.line.unwrap().stack_after, vec![Ty::Str]);
}

#[test]
fn definitions_and_lines_do_not_mix() {
    let mut s = session();
    assert_eq!(code(&mut s, ": f ( -- ) ; 3"), "E_SYNTAX");
}

#[test]
fn tests_run_when_their_word_has_a_body() {
    let mut s = session();
    ok(&mut s, ": sq ( i32 -- i32 ) dup i32.mul ;");
    let step = ok(&mut s, "test sq : 3 sq -> 9");
    assert_eq!(step.tests.len(), 1);
    assert_eq!(step.tests[0].word, "sq");
    assert_eq!(step.tests[0].expected, vec![Value::I32(9)]);
    assert_eq!(step.tests[0].types, vec!["i32"]);
    assert!(step.installs.iter().any(|i| i.slot == step.tests[0].slot));

    ok(&mut s, "declare h ( -- i32 )");
    assert!(ok(&mut s, "test h : h -> 1").tests.is_empty());
    let step = ok(&mut s, ": h ( -- i32 ) 1 ;");
    assert_eq!(step.tests.len(), 1);
    assert_eq!(step.tests[0].word, "h");

    assert_eq!(ok(&mut s, "test i32.add : 1 2 i32.add -> 3").tests.len(), 1);
    let step = s.step("test sq : 3 sq -> 9 i64", 0x20_0000);
    assert_eq!(step.diagnostics[0].code, "E_TEST_TYPE");
    assert!(step.tests.is_empty());

    let step = ok(&mut s, ": sq ( i32 -- i32 ) 3 i32.mul ;");
    assert_eq!(step.tests.len(), 1);
    assert_eq!(step.tests[0].word, "sq");
}

#[test]
fn struct_declaration_step() {
    let mut s = session();
    let step = ok(&mut s, "struct point  x: i32  y: f64");
    assert_eq!(step.defined.len(), 5);
    assert_eq!(step.installs.len(), 5);
    assert!(step.module.is_some());
    let again = ok(&mut s, "struct point  x: i32  y: f64");
    assert!(again.defined.is_empty());
    assert!(again.module.is_none());
}

fn has(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn struct_values_cross_steps() {
    let mut plain = session();
    let step = ok(&mut plain, "1 2 i32.add");
    assert!(!has(step.module.as_ref().unwrap(), b"refs"));
    assert_eq!(step.refs_size, 0);

    let mut s = session();
    ok(&mut s, "struct point  x: i32  y: f64");
    let p = Ty::Struct("point".into());
    let step = ok(&mut s, "7 2.5 point.new");
    assert_eq!(step.line.as_ref().unwrap().stack_after, vec![p.clone()]);
    assert_eq!(step.refs_size, 1);
    assert!(has(step.module.as_ref().unwrap(), b"refs"));

    s.stack = vec![p.clone()];
    let step = ok(&mut s, "point.x");
    assert_eq!(step.line.as_ref().unwrap().stack_after, vec![Ty::I32]);

    s.stack = vec![];
    let step = ok(&mut s, "3 array.new ( array point )");
    assert_eq!(step.refs_size, 3);

    s.stack = vec![Ty::Array(Box::new(p))];
    let step = ok(&mut s, "array.len");
    assert_eq!(step.line.as_ref().unwrap().stack_after, vec![Ty::I32]);
}

#[test]
fn forget_refuses_while_used() {
    let mut s = session();
    ok(&mut s, ": sq ( i32 -- i32 ) dup i32.mul ;");
    ok(&mut s, ": quad ( i32 -- i32 ) sq sq ;");
    ok(
        &mut s,
        ": maybe ( i32 -- i32 ) dup 0 i32.gt_s [ sq ] [ ] if ;",
    );
    ok(&mut s, ": one ( -- i32 ) 1 ;");
    ok(&mut s, "test one : one sq -> 1");
    ok(&mut s, "3 sq");
    let step = s.step(")forget sq", 0x20_0000);
    assert_eq!(step.diagnostics[0].code, "E_FORGET");
    assert_eq!(
        step.diagnostics[0].dependants,
        Some(vec!["maybe".into(), "quad".into(), "test one".into()])
    );
    assert!(step.forgotten.is_empty());
    for w in ["quad", "maybe", "one", "sq"] {
        assert_eq!(ok(&mut s, &format!(")forget {w}")).forgotten, vec![w]);
    }
    assert_eq!(code(&mut s, "sq"), "E_UNDEFINED");
}

#[test]
fn forget_frees_the_name_and_drops_tests() {
    let mut s = session();
    ok(&mut s, ": sq ( i32 -- i32 ) dup i32.mul ;");
    assert_eq!(ok(&mut s, "test sq : 3 sq -> 9").tests.len(), 1);
    ok(&mut s, ")forget sq");
    let step = ok(&mut s, ": sq ( f64 -- f64 ) dup f64.mul ;");
    assert!(step.tests.is_empty(), "the old test must not run again");
    assert!(ok(&mut s, "1.5 sq").line.is_some());
}

#[test]
fn forget_refuses_primitives_prelude_and_struct_words() {
    let mut s = session();
    ok(&mut s, "struct point  x: i32  y: f64");
    for w in ["dup", "i64", "point.x", "point.new"] {
        assert_eq!(code(&mut s, &format!(")forget {w}")), "E_FORGET", "{w}");
    }
    assert_eq!(code(&mut s, ")forget nothing"), "E_UNDEFINED");
    assert_eq!(code(&mut s, ")frob"), "E_SYNTAX");
}
