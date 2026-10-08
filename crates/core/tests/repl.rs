use wack_core::layout::LITERALS_BASE;
use wack_core::repl::Forced;
use wack_core::types::{Effect, Ty};
use wack_core::{validate, Session, Step, Value};

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
    let (s, step) = Session::new(true, false, LITERALS_BASE, false);
    // The REPL processes the prelude eagerly, so this is what checks the
    // prelude's generic templates (whole programs make them lazily, unchecked).
    assert!(step.ok(), "{:?}", step.diagnostics);
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
    let p = Ty::Struct("point".into(), Vec::new());
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

fn imports(wasm: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        if let wasmparser::Payload::ImportSection(r) = payload.unwrap() {
            for i in r.into_imports() {
                let i = i.unwrap();
                out.push(format!("{}.{}", i.module, i.name));
            }
        }
    }
    out
}

#[test]
fn spawn_imports_its_global_from_its_step_on() {
    let mut s = session();
    let step = ok(&mut s, "1 2 i32.add");
    assert!(!imports(step.module.as_ref().unwrap()).contains(&"wack.spawn".to_string()));
    let step = ok(&mut s, ": f ( -- ) [ \"x\" println ] spawn ;");
    assert!(imports(step.module.as_ref().unwrap()).contains(&"wack.spawn".to_string()));
}

#[test]
fn alt_in_a_line() {
    let mut s = session();
    ok(&mut s, "chan.make ( chan i32 ) :> c c recv: [ drop ] alt");
}

#[test]
fn function_values_cross_steps() {
    let mut s = session();
    ok(&mut s, ": inc ( i32 -- i32 ) 1 i32.add ;");
    let step = ok(&mut s, "'inc");
    let q = Ty::Quot(Box::new(Effect::new(vec![Ty::I32], vec![Ty::I32])));
    assert_eq!(step.line.as_ref().unwrap().stack_after, vec![q.clone()]);
    assert_eq!(step.refs_size, 1);
    assert!(has(step.module.as_ref().unwrap(), b"refs"));
    s.stack = vec![q];
    let step = ok(&mut s, "5 swap call");
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

#[test]
fn force_chunk_continues_until_a_blank_line() {
    use wack_core::repl::needs_more;
    assert!(needs_more(")force : f ( -- i32 ) 1 ;\n"));
    assert!(!needs_more(
        ")force : f ( -- i32 ) 1 ;\n: g ( -- ) f drop ;\n\n"
    ));
    assert!(!needs_more(")forget f\n"));
}

fn force_setup() -> Session {
    let mut s = session();
    ok(&mut s, ": f ( -- i32 ) 1 ;");
    ok(&mut s, ": g ( -- ) f drop ;");
    ok(&mut s, ": h ( -- i32 ) f ;");
    ok(&mut s, "test h : h -> 1");
    s
}

#[test]
fn force_refuses_and_changes_nothing() {
    let mut s = force_setup();
    let step = s.step(")force : f ( -- i64 ) 1 i64 ;", 0x20_0000);
    assert_eq!(step.diagnostics[0].code, "E_FORCE");
    assert_eq!(step.diagnostics[0].dependants, Some(vec!["h".to_string()]));
    assert!(step.diagnostics.len() > 1, "the breaking error follows");
    assert!(step.module.is_none());
    assert!(step.forced.is_empty());
    let line = ok(&mut s, "f").line.unwrap();
    assert_eq!(line.stack_after, vec![Ty::I32]);
}

#[test]
fn force_commits_atomically_with_a_fresh_slot() {
    let mut s = force_setup();
    let old_f = s.word_slot("f").unwrap();
    let old_g = s.word_slot("g").unwrap();
    let old_h = s.word_slot("h").unwrap();
    let step = ok(
        &mut s,
        ")force : f ( -- i64 ) 1 i64 ;\n: h ( -- i32 ) f i32.wrap_i64 ;\n",
    );
    assert_eq!(
        step.forced,
        vec![Forced {
            name: "f".into(),
            from: "( -- i32 )".into(),
            to: "( -- i64 )".into()
        }]
    );
    assert_eq!(step.rechecked, vec!["g".to_string()]);
    let defined: Vec<&str> = step.defined.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(defined, ["f", "h"]);
    let new_f = s.word_slot("f").unwrap();
    assert!(new_f > old_f);
    let slots: Vec<u32> = step.installs.iter().map(|i| i.slot).collect();
    assert!(
        slots.contains(&new_f) && !slots.contains(&old_f),
        "{slots:?}"
    );
    assert!(
        slots.contains(&old_g) && slots.contains(&old_h),
        "{slots:?}"
    );
    assert_eq!(s.word_slot("g"), Some(old_g));
    assert!(step.tests.iter().any(|t| t.word == "h"));
    let line = ok(&mut s, "f").line.unwrap();
    assert_eq!(line.stack_after, vec![Ty::I64]);
}

#[test]
fn force_drops_the_words_own_tests() {
    let mut s = session();
    ok(&mut s, ": f ( -- i32 ) 1 ;");
    ok(&mut s, "test f : f -> 1");
    let step = ok(&mut s, ")force : f ( -- i64 ) 1 i64 ;\n");
    assert!(step.tests.is_empty(), "the old contract's test is dropped");
    let step = ok(&mut s, ")force : f ( -- i32 ) 2 ;\ntest f : f -> 2\n");
    assert_eq!(step.tests.len(), 1);
    assert_eq!(step.tests[0].word, "f");
}

#[test]
fn force_refusals() {
    let mut s = session();
    assert_eq!(code(&mut s, ")force : dup ( -- ) ;"), "E_FORCE");
    assert_eq!(code(&mut s, ")force : println ( -- ) ;"), "E_FORCE");
    assert_eq!(code(&mut s, ")force : nothing ( -- ) ;"), "E_UNDEFINED");
    ok(&mut s, ": q ( -- ) ;");
    assert_eq!(code(&mut s, ")force declare q ( -- )"), "E_SYNTAX");
    ok(&mut s, "struct point  x: i32  y: f64");
    assert_eq!(
        code(&mut s, ")force : point.x ( point -- i32 ) ;"),
        "E_FORCE"
    );
}

#[test]
fn recursive_words_must_declare_at_the_repl() {
    let mut s = session();
    let step = s.step(
        ": fact dup 1 i32.gt_s [ dup 1 i32.sub fact i32.mul ] when ;",
        0x20_0000,
    );
    assert_eq!(step.diagnostics[0].code, "E_NEEDS_EFFECT");
    assert!(step.defined.is_empty());
}

#[test]
fn inferred_definition_at_the_repl() {
    let mut s = session();
    let step = ok(&mut s, ": sq dup i32.mul ;");
    assert_eq!(step.defined[0].effect, "( i32 -- i32 )");
    assert!(step.defined[0].inferred);
}

#[test]
fn redefining_a_generic_reinstalls_its_instances() {
    let mut s = session();
    ok(&mut s, ": twice ( T -- T T ) dup ;");
    ok(&mut s, ": a ( i32 -- i32 i32 ) twice ;");
    ok(&mut s, "declare g ( str -- str )");
    ok(&mut s, ": b ( str -- str str ) twice ;");
    let step = ok(&mut s, ": twice ( T -- T T ) dup ;");
    assert_eq!(step.installs.len(), 3, "{:?}", step.installs);
    assert_eq!(step.defined.len(), 1);
}

#[test]
fn lines_and_tests_install_generic_instances() {
    let mut s = session();
    let step = ok(&mut s, ": twice ( T -- T T ) dup ;");
    assert_eq!(step.defined[0].effect, "( T -- T T )");
    let step = ok(&mut s, "3 twice");
    assert_eq!(
        step.line.as_ref().unwrap().stack_after,
        vec![Ty::I32, Ty::I32]
    );
    assert_eq!(
        step.installs.len(),
        2,
        "the instance and the line: {:?}",
        step.installs
    );
    s.stack = vec![Ty::I32, Ty::I32];
    let step = ok(&mut s, "drop drop \"a\" twice");
    assert_eq!(step.installs.len(), 2, "a second instance and the line");
    assert_eq!(step.line.unwrap().stack_after, vec![Ty::Str, Ty::Str]);
    assert_eq!(ok(&mut s, "test twice : 3 twice -> 3 3").tests.len(), 1);
}

fn generic_setup() -> Session {
    let mut s = session();
    ok(&mut s, ": twice ( T -- T T ) dup ;");
    ok(&mut s, ": a ( i32 -- i32 i32 ) twice ;");
    ok(&mut s, "test a : 1 a -> 1 1");
    s
}

#[test]
fn forget_a_generic_word() {
    let mut s = generic_setup();
    let step = s.step(")forget twice", 0x20_0000);
    assert_eq!(step.diagnostics[0].code, "E_FORGET");
    assert_eq!(step.diagnostics[0].dependants, Some(vec!["a".to_string()]));
    assert_eq!(code(&mut s, ")forget twice<i32>"), "E_UNDEFINED");
    ok(&mut s, ")forget a");
    assert_eq!(ok(&mut s, ")forget twice").forgotten, vec!["twice"]);
    let step = ok(&mut s, ": twice ( i32 -- i32 ) ;");
    assert_eq!(step.defined[0].effect, "( i32 -- i32 )");
}

#[test]
fn force_a_generic_word() {
    let mut s = generic_setup();
    let old = s.word_slot("twice").unwrap();
    let step = s.step(")force : twice ( T -- T ) ;\n", 0x20_0000);
    assert_eq!(step.diagnostics[0].code, "E_FORCE");
    assert_eq!(step.diagnostics[0].dependants, Some(vec!["a".to_string()]));
    let step = ok(
        &mut s,
        ")force : twice ( T -- T ) ;\n: a ( i32 -- i32 i32 ) twice dup ;\n",
    );
    assert_eq!(step.forced[0].to, "( T -- T )");
    assert!(
        step.rechecked.is_empty(),
        "a was redefined in the chunk, not rechecked"
    );
    assert!(s.word_slot("twice").unwrap() > old);
    let new_slots: Vec<u32> = step.installs.iter().map(|i| i.slot).collect();
    assert!(
        new_slots.len() >= 3,
        "template, a and a fresh instance: {new_slots:?}"
    );
    let mut s = session();
    ok(&mut s, ": f ( i32 -- i32 i32 ) dup ;");
    assert_eq!(
        ok(&mut s, ")force : f dup ;\n").forced[0].to,
        "( T -- T T )"
    );
}

#[test]
fn unions_at_the_repl() {
    let mut s = session();
    let step = ok(
        &mut s,
        "union shape | circle  r: f64 | rect  w: f64  h: f64 | empty",
    );
    assert_eq!(step.defined.len(), 7, "3 constructors, tag and 3 readers");
    assert_eq!(step.installs.len(), 7);
    let step = ok(&mut s, "1.0 shape.circle");
    assert_eq!(step.refs_size, 1);
    let module = step.module.unwrap();
    assert!(
        wasmparser::Parser::new(0).parse_all(&module).any(|p| matches!(
            p,
            Ok(wasmparser::Payload::ImportSection(r)) if r.clone().into_imports().any(|i| i.unwrap().name == "refs")
        )),
        "a session with a union imports wack.refs"
    );
    assert_eq!(code(&mut s, ")forget shape.circle"), "E_FORGET");
}

#[test]
fn prelude_option_at_the_repl() {
    let mut s = session();
    let step = ok(&mut s, "3 option.some");
    assert_eq!(step.refs_size, 1);
    assert_eq!(
        step.line.as_ref().unwrap().stack_after[0].to_string(),
        "option i32"
    );
}

#[test]
fn eq_on_structs_at_the_repl() {
    let mut s = session();
    ok(&mut s, "struct point  x: i32  y: f64");
    let step = ok(&mut s, "1 2.0 point.new 1 2.0 point.new eq");
    assert_eq!(step.line.as_ref().unwrap().stack_after, vec![Ty::I32]);
    assert!(step.installs.len() > 1, "the line and eq<point>");
}

#[test]
fn vec_at_the_repl() {
    let mut s = session();
    let step = ok(&mut s, "vec.make ( vec i32 ) dup 3 vec.push vec.len");
    assert_eq!(step.line.as_ref().unwrap().stack_after, vec![Ty::I32]);
}

#[test]
fn words_lists_the_program_as_it_stands() {
    let mut s = session();
    ok(&mut s, ": L 100 i32.add ;");
    ok(&mut s, ")forget L");
    ok(&mut s, ": L ( i32 i32 -- i32 ) i32.sub ;  # turn left");
    ok(&mut s, ": P1 ( -- i32 ) 50 68 L ;");
    ok(&mut s, ": L ( i32 i32 -- i32 ) i32.sub 100 i32.rem_u ;");
    ok(&mut s, "struct point  x: i32  y: i32");
    ok(&mut s, "declare later ( -- i32 )");
    ok(&mut s, "test P1 : P1 -> 82\ntest later : later -> 1");
    ok(&mut s, "1 2 L");
    assert!(s.step(": bad ( -- i32 ) nope ;", 0x20_0000).diagnostics[0].is_error());
    let listing = ok(&mut s, ")words").listing.unwrap();
    assert_eq!(
        listing,
        "struct point  x: i32  y: i32\n\
         \n\
         declare later ( -- i32 )\n\
         \n\
         : L ( i32 i32 -- i32 ) i32.sub 100 i32.rem_u ;\n\
         : P1 ( -- i32 ) 50 68 L ;\n\
         \n\
         test P1 : P1 -> 82\n\
         test later : later -> 1\n"
    );
    // It reads back as one chunk.
    ok(&mut session(), &listing);
}

#[test]
fn words_follows_force() {
    let mut s = force_setup();
    ok(
        &mut s,
        ")force : f ( -- i64 ) 1 i64 ;\n: h ( -- i32 ) f i32.wrap_i64 ;\n",
    );
    let listing = ok(&mut s, ")words").listing.unwrap();
    assert_eq!(
        listing,
        ": f ( -- i64 ) 1 i64 ;\n\
         : g ( -- ) f drop ;\n\
         : h ( -- i32 ) f i32.wrap_i64 ;\n\
         \n\
         test h : h -> 1\n"
    );
    ok(&mut session(), &listing);
}

#[test]
fn transformed_sessions_validate() {
    let (mut s, step) = Session::new(true, false, LITERALS_BASE, true);
    assert!(step.ok(), "{:?}", step.diagnostics);
    let m = step.module.as_ref().unwrap();
    validate(m).unwrap();
    let mut imports = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(m) {
        if let wasmparser::Payload::ImportSection(r) = payload.unwrap() {
            for i in r.into_imports() {
                let i = i.unwrap();
                imports.push(format!("{}.{}", i.module, i.name));
            }
        }
    }
    assert!(imports.iter().any(|i| i == "wack.frames"), "{imports:?}");
    for text in [
        ": r ( chan i32 -- i32 ) chan.recv none: [ 0 ] some: [ ] match ;",
        ": sends ( chan i32 i32 -- ) :> n :> c  n [ c swap chan.send ] times ;",
        ": either ( chan i32 chan i32 -- i32 ) :> b :> a \
         a recv: [ none: [ 0 ] some: [ ] match ] b recv: [ none: [ 1 ] some: [ ] match ] alt ;",
        ": opt ( option i32 -- i32 ) none: [ 0 ] some: [ 1 i32.add ] match ;",
        "chan.make ( chan i32 ) :> c  [ c 3 sends ] spawn",
        ": sq ( i32 -- i32 ) dup i32.mul ;",
        "3 sq",
    ] {
        // `ok` validates each step's module.
        ok(&mut s, text);
    }
}

/// Every operator of a step module's code: (call_indirect count, whether the
/// i32 constant `marker` appears).
fn step_calls(step: &Step, marker: i32) -> (usize, bool) {
    use wasmparser::{Operator, Parser, Payload};
    let (mut indirect, mut seen) = (0, false);
    for p in Parser::new(0).parse_all(step.module.as_ref().unwrap()) {
        if let Payload::CodeSectionEntry(body) = p.unwrap() {
            let mut r = body.get_operators_reader().unwrap();
            while !r.eof() {
                match r.read().unwrap() {
                    Operator::CallIndirect { .. } => indirect += 1,
                    Operator::I32Const { value } if value == marker => seen = true,
                    _ => {}
                }
            }
        }
    }
    (indirect, seen)
}

/// The REPL calls words through the table (`indirect_calls`), so the compiler
/// never inlines there and a redefinition reaches existing callers. The core
/// session compiles steps without running them: this checks that `g`'s step
/// reaches `f` through the table with none of `f`'s code copied in (`f`'s
/// marker constants), and that the slot is kept across a redefinition; the
/// check.rs unit test `not_inlined_in_the_repl` covers the emitter's side.
#[test]
fn redefinition_reaches_callers_of_small_words() {
    let mut s = session();
    let f = ok(&mut s, ": f ( -- i32 ) 4241 ;").installs[0].slot;
    let g = ok(&mut s, ": g ( -- i32 ) f ;");
    let (indirect, copied) = step_calls(&g, 4241);
    assert!(indirect >= 1, "g calls f through the table");
    assert!(!copied, "f's body is not spliced into g");
    let step = ok(&mut s, ": f ( -- i32 ) 4242 ;");
    assert_eq!(
        step.installs[0].slot, f,
        "the redefinition keeps f's slot, so g sees it"
    );
    let line = ok(&mut s, "g");
    assert!(
        !step_calls(&line, 4242).1,
        "the line calls g, which calls f, through the table"
    );
    assert_eq!(ok(&mut s, ")forget g").forgotten, vec!["g".to_string()]);
    assert_eq!(ok(&mut s, ")forget f").forgotten, vec!["f".to_string()]);
    ok(&mut s, ": f ( -- i32 ) 4243 ;");
    let g = ok(&mut s, ": g ( -- i32 ) f ;");
    let (indirect, copied) = step_calls(&g, 4243);
    assert!(indirect >= 1 && !copied);
}
