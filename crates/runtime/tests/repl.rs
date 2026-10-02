use chasm_core::repl::StackEntry;
use chasm_core::Value;
use chasm_runtime::namespace::{Config, Console};
use chasm_runtime::native::TestStatus;
use chasm_runtime::repl::{NativeRepl, Outcome};

fn repl() -> NativeRepl {
    NativeRepl::new(
        Config {
            console: Console::Capture {
                input: vec![],
                pos: 0,
                output: vec![],
            },
            file: false,
            mounts: Default::default(),
            net: false,
        },
        true,
    )
    .unwrap()
}

fn e(ty: &str, value: &str) -> StackEntry {
    StackEntry {
        ty: ty.into(),
        value: value.into(),
    }
}

fn ok(r: &mut NativeRepl, text: &str) -> Outcome {
    let o = r.step(text);
    for d in &o.diagnostics {
        eprintln!("{}", d.render());
    }
    assert!(o.diagnostics.is_empty(), "`{text}` had diagnostics");
    assert!(o.trap.is_none(), "`{text}` trapped: {:?}", o.trap);
    o
}

fn trap(r: &mut NativeRepl, text: &str) -> Outcome {
    let o = r.step(text);
    assert!(o.diagnostics.is_empty());
    assert!(o.trap.is_some(), "`{text}` should trap");
    o
}

#[test]
fn lines_definitions_traps_and_redefinition() {
    let mut r = repl();
    let o = ok(&mut r, ": sq ( i32 -- i32 ) dup i32.mul ;");
    assert_eq!(o.defined[0].name, "sq");
    assert_eq!(ok(&mut r, "3 sq").stack, vec![e("i32", "9")]);

    let o = ok(&mut r, "\"hi\" println");
    assert!(r.host().captured_output().ends_with(b"hi\n"));
    assert_eq!(o.stack, vec![e("i32", "9")]);

    let o = trap(&mut r, "1 0 i32.div_s");
    assert!(o.trap.unwrap().message.contains("divide by zero"));
    assert_eq!(o.stack, vec![e("i32", "9")]);

    let o = trap(&mut r, "\"boom\" trap");
    let t = o.trap.unwrap();
    assert_eq!(t.message, "boom");
    assert!(t.word.unwrap().starts_with("[line "));
    assert_eq!(o.stack, vec![e("i32", "9")]);

    ok(&mut r, ": twice ( i32 -- i32 ) sq sq ;");
    ok(&mut r, ": sq ( i32 -- i32 ) 2 i32.mul ;");
    assert_eq!(ok(&mut r, "drop 3 twice").stack, vec![e("i32", "12")]);

    ok(&mut r, "declare g ( -- i32 )");
    let o = trap(&mut r, "g");
    assert_eq!(o.trap.unwrap().message, "unresolved word g");
    assert_eq!(o.stack, vec![e("i32", "12")]);

    let o = r.step("i32.add");
    assert_eq!(o.diagnostics[0].code, "E_STACK_UNDERFLOW");
    assert!(o.trap.is_none());
    assert_eq!(o.stack, vec![e("i32", "12")]);

    let o = ok(&mut r, "drop \"a\" \"b\" str.concat");
    assert_eq!(o.stack, vec![e("str", "\"ab\"")]);
    let _ = (
        o.timing.compile_us,
        o.timing.instantiate_us,
        o.timing.run_us,
    );
}

#[test]
fn tests_run_in_the_shared_instance() {
    let mut r = repl();
    ok(&mut r, ": sq ( i32 -- i32 ) dup i32.mul ;");
    let o = ok(&mut r, "test sq : 3 sq -> 9");
    assert_eq!(o.tests.len(), 1);
    assert_eq!(o.tests[0].status, TestStatus::Pass);

    let o = ok(&mut r, "test sq : 3 sq -> 10");
    assert_eq!(o.tests[0].status, TestStatus::Fail);
    assert_eq!(o.tests[0].actual, Some(vec![Value::I32(9)]));

    ok(&mut r, "declare cube ( i32 -- i32 )");
    assert!(ok(&mut r, "test cube : 2 cube -> 8").tests.is_empty());
    let o = ok(&mut r, ": cube ( i32 -- i32 ) dup dup i32.mul i32.mul ;");
    assert_eq!(o.tests.len(), 1);
    assert_eq!(o.tests[0].word, "cube");
    assert_eq!(o.tests[0].status, TestStatus::Pass);

    // A test whose body traps at run time fails with the trap.
    let o = ok(&mut r, "test sq : 1 0 i32.div_s sq -> 9");
    assert_eq!(o.tests[0].status, TestStatus::Fail);
    assert!(o.tests[0].error.is_some());
}
