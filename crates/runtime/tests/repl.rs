use wack_core::repl::StackEntry;
use wack_core::Value;
use wack_runtime::namespace::{Config, Console};
use wack_runtime::native::TestStatus;
use wack_runtime::repl::{NativeRepl, Outcome};

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

fn output(r: &NativeRepl) -> String {
    String::from_utf8_lossy(r.host().captured_output()).into_owned()
}

#[test]
fn a_line_receives_from_a_process_it_spawned() {
    let mut r = repl();
    let o = ok(
        &mut r,
        "chan.make ( chan i32 ) :> c  c [ c 1 chan.send  c chan.close ] spawn  c chan.recv",
    );
    assert_eq!(o.stack.last(), Some(&e("option i32", "option.some{v: 1}")));
}

#[test]
fn a_process_parked_in_one_step_runs_in_a_later_one() {
    let mut r = repl();
    ok(&mut r, ": schan ( -- chan str ) chan.make ;");
    let o = ok(
        &mut r,
        "schan :> d  d  [ d chan.recv none: [ \"closed\" println ] some: [ println ] match ] spawn",
    );
    assert_eq!(o.stack.last().map(|s| s.ty.as_str()), Some("chan str"));
    assert!(!output(&r).contains("later"));
    ok(&mut r, "dup \"later\" chan.send");
    assert!(output(&r).contains("later\n"), "{}", output(&r));
}

#[test]
fn a_line_that_can_never_finish_traps_and_keeps_the_stack() {
    let mut r = repl();
    ok(&mut r, ": ichan ( -- chan i32 ) chan.make ;");
    ok(&mut r, "7");
    let o = trap(&mut r, "ichan chan.recv");
    let t = o.trap.unwrap();
    assert!(
        t.message
            .starts_with("all processes blocked: [line] waits to receive on chan"),
        "{}",
        t.message
    );
    assert_eq!(t.word.as_deref(), Some("[line]"));
    assert_eq!(o.stack, vec![e("i32", "7")], "stack unchanged");
    assert_eq!(ok(&mut r, "1 i32.add").stack, vec![e("i32", "8")]);
}

#[test]
fn a_test_using_channels_passes() {
    let mut r = repl();
    ok(
        &mut r,
        ": t ( -- i32 ) chan.make ( chan i32 ) :> c  [ c 5 chan.send ] spawn  \
         c chan.recv some: [ ] none: [ 0 ] match ;",
    );
    let o = r.step("test t : t -> 5");
    assert!(o.diagnostics.is_empty());
    assert_eq!(
        o.tests[0].status,
        TestStatus::Pass,
        "{:?}",
        o.tests[0].error
    );
}

#[test]
fn alt_takes_the_channel_with_a_value() {
    let mut r = repl();
    ok(&mut r, ": ichan ( -- chan i32 ) chan.make ;");
    let o = ok(
        &mut r,
        "ichan :> a  ichan :> b  [ a 3 chan.send ] spawn  [ b 7 chan.send ] spawn  \
         a recv: [ none: [ 0 ] some: [ ] match ] b recv: [ none: [ 0 ] some: [ ] match ] alt",
    );
    let top = o.stack.last().unwrap();
    assert!(top == &e("i32", "3") || top == &e("i32", "7"), "{top:?}");
}

#[test]
fn a_killed_receiver_never_runs() {
    let mut r = repl();
    ok(&mut r, ": schan ( -- chan str ) chan.make ;");
    ok(
        &mut r,
        "schan :> e  e  [ e chan.recv drop \"zombie\" println ] spawn",
    );
    let before = output(&r).len();
    ok(&mut r, "\"/prog\" ls drop");
    let listing = output(&r)[before..].to_string();
    let pids: Vec<u32> = listing
        .split_whitespace()
        .filter_map(|x| x.trim_end_matches('/').parse().ok())
        .collect();
    let victim = *pids.iter().max().unwrap();
    assert!(pids.contains(&0) && victim > 0, "ls /prog: {listing:?}");
    ok(
        &mut r,
        &format!("\"kill\" \"/prog/{victim}/ctl\" write-file drop"),
    );
    ok(&mut r, "dup chan.close drop");
    assert!(!output(&r).contains("zombie"), "{}", output(&r));
}

#[test]
fn a_trap_in_a_spawned_process_ends_only_that_process() {
    let mut r = repl();
    ok(&mut r, ": boom ( -- ) \"bad\" trap ;");
    let o = ok(&mut r, "[ boom ] spawn  1 2 i32.add");
    assert_eq!(o.stack, vec![e("i32", "3")]);
    assert_eq!(o.process_traps.len(), 1);
    let t = &o.process_traps[0];
    assert_eq!(t.process, Some(1));
    assert_eq!(t.word.as_deref(), Some("boom"));
    assert_eq!(t.message, "bad");
    assert_eq!(ok(&mut r, "4").stack, vec![e("i32", "3"), e("i32", "4")]);
}

#[test]
fn plain_lines_stay_fast() {
    let mut r = repl();
    ok(&mut r, ": sq ( i32 -- i32 ) dup i32.mul ;");
    let o = ok(&mut r, "3 sq");
    assert_eq!(o.stack, vec![e("i32", "9")]);
    assert!(o.timing.run_us < 1000, "{} us", o.timing.run_us);
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

#[test]
fn union_values_echo() {
    let mut r = repl();
    assert!(r
        .step("union shape | circle  r: f64 | rect  w: f64  h: f64 | empty")
        .diagnostics
        .is_empty());
    let o = r.step("1.5 shape.circle");
    assert_eq!(o.stack, vec![e("shape", "shape.circle{r: 1.5}")]);
}
