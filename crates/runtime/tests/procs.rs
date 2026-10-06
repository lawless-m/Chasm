//! Processes run natively: transformed modules unwind when a process must
//! wait and rewind when it resumes (M12).
use wack_core::{compile, Compilation, Options, Source};
use wack_runtime::namespace::{Config, Console};
use wack_runtime::native::{RunError, Runner};

fn config() -> Config {
    Config {
        console: Console::Capture {
            input: vec![],
            pos: 0,
            output: vec![],
        },
        file: false,
        mounts: Default::default(),
        net: false,
    }
}

fn build(src: &str) -> Compilation {
    let c = compile(
        &[Source::new("t.wack", src)],
        &Options {
            prelude: true,
            test_exports: false,
            export: true,
            wasi: false,
        },
    );
    assert!(c.ok(), "{:?}", c.diagnostics);
    c
}

/// Run `main`: its result and the captured console.
fn run(src: &str) -> (Result<(), RunError>, String) {
    let c = build(src);
    let runner = Runner::new(c.wasm.as_ref().unwrap()).unwrap();
    let o = runner.run_main(config());
    let out = String::from_utf8_lossy(o.host.captured_output()).into_owned();
    (o.result, out)
}

fn example(name: &str) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join("examples").join(name)).unwrap()
}

#[test]
fn ping_pong_and_alt() {
    let (r, out) = run(&example("alt.wack"));
    r.unwrap();
    assert_eq!(out, "5 5\n8\n");
}

#[test]
fn two_producers_feed_one_reader() {
    let (r, out) = run(&example("pipeline.wack"));
    r.unwrap();
    assert_eq!(out, "110\n");
}

#[test]
fn a_forgotten_close_is_reported_as_blocked() {
    let (r, _) = run(
        ": main ( -- )\n  chan.make ( chan i32 ) :> c\n  [ c 1 chan.send ] spawn\n  \
         c chan.recv drop  c chan.recv drop ;",
    );
    let e = r.unwrap_err();
    assert!(
        e.message
            .starts_with("all processes blocked: main waits to receive on chan 1"),
        "{}",
        e.message
    );
    assert_eq!(e.word.as_deref(), Some("main"));
    assert_eq!(e.process, None);
}

#[test]
fn a_trap_in_a_spawned_process_ends_the_run() {
    let (r, _) = run(": boom ( -- ) \"bad\" trap ;\n\
         : main ( -- ) [ boom ] spawn  chan.make ( chan i32 ) chan.recv drop ;");
    let e = r.unwrap_err();
    assert_eq!(e.message, "bad");
    assert_eq!(e.word.as_deref(), Some("boom"));
    assert_eq!(e.process, Some(1));
    assert_eq!(e.to_string(), "trap in `boom` (process 1): bad");
}

#[test]
fn spawned_processes_run_after_main_returns() {
    let (r, out) = run(": main ( -- ) [ \"later\" println ] spawn  \"first\" println ;");
    r.unwrap();
    assert_eq!(out, "first\nlater\n");
}

#[test]
fn a_parked_process_keeps_its_struct_through_collection() {
    let (r, out) = run("struct box  v: i32\n\
         : churn ( i32 -- ) [ drop 1 box.new drop ] times ;\n\
         : main ( -- )\n  chan.make ( chan i32 ) :> g  42 box.new :> bx\n  \
         [ bx :> mine  g chan.recv drop  mine box.v i32.to-str println ] spawn\n  \
         [ 2000000 churn  g 0 chan.send ] spawn ;");
    r.unwrap();
    assert_eq!(out, "42\n");
}

#[test]
fn programs_without_processes_run_as_before() {
    let c = build(": main ( -- ) \"plain\" println ;");
    assert!(c.processes.is_none());
    let (r, out) = run(": main ( -- ) \"plain\" println ;");
    r.unwrap();
    assert_eq!(out, "plain\n");
}

#[test]
fn prog_lists_live_processes_and_kills_a_parked_one() {
    let (r, out) = run(
        ": main ( -- )\n  chan.make ( chan i32 ) :> c  chan.make ( chan i32 ) :> d\n  \
         [ c chan.recv drop \"zombie\" println ] spawn\n  \
         [ d 1 chan.send ] spawn\n  \
         d chan.recv drop\n  \
         \"/prog\" ls drop\n  \
         \"kill\" \"/prog/1/ctl\" write-file i32.to-str println\n  \
         c chan.close  \"done\" println ;",
    );
    r.unwrap();
    assert_eq!(out, "0/\n1/\n0\ndone\n", "the killed receiver never ran");
}

#[test]
fn prog_ctl_refuses_what_it_does_not_know() {
    let (r, out) = run(
        ": main ( -- )\n  chan.make ( chan i32 ) :> c\n  [ c chan.recv drop ] spawn\n  \
         \"stop\" \"/prog/1/ctl\" write-file i32.to-str println\n  \
         \"kill\" \"/prog/7/ctl\" write-file i32.to-str println\n  \
         c chan.close ;",
    );
    r.unwrap();
    assert_eq!(out, "-6\n-1\n");
}

#[test]
fn a_process_that_kills_itself_stops_there() {
    let (r, out) = run(": main ( -- )\n  \
         [ \"kill\" \"/prog/1/ctl\" write-file drop \"not printed\" println ] spawn\n  \
         [ \"after\" println ] spawn\n  \
         \"main goes on\" println ;");
    r.unwrap();
    assert_eq!(out, "main goes on\nafter\n");
}

fn tests_of(src: &str) -> Vec<wack_runtime::native::TestResult> {
    let c = compile(
        &[Source::new("t.wack", src)],
        &Options {
            prelude: true,
            test_exports: true,
            export: false,
            wasi: false,
        },
    );
    assert!(c.ok(), "{:?}", c.diagnostics);
    wack_runtime::native::run_tests(&c, &config()).unwrap()
}

#[test]
fn channel_tests_run_as_process_0() {
    use wack_runtime::native::TestStatus;
    let r = tests_of(&example("pipeline.wack"));
    assert!(r.len() == 2 && r.iter().all(|t| t.status == TestStatus::Pass));

    let forgot = example("pipeline.wack").replace("  c chan.close ;", "  ;");
    let r = tests_of(&forgot);
    let sum = r.iter().find(|t| t.test.word == "sum-of").unwrap();
    assert_eq!(sum.status, TestStatus::Fail);
    let e = sum.error.as_ref().unwrap();
    assert!(
        e.message.starts_with("all processes blocked: sum-of waits"),
        "{}",
        e.message
    );

    let r = tests_of(
        ": boom ( -- ) \"bad\" trap ;\n\
         : t ( -- i32 ) [ boom ] spawn  chan.make ( chan i32 ) chan.recv drop 1 ;\n\
         test t : t -> 1",
    );
    assert_eq!(r[0].status, TestStatus::Fail);
    assert_eq!(r[0].error.as_ref().unwrap().process, Some(1));
}

#[test]
fn a_sleeper_serves_process_0() {
    let (r, out) = run(": main ( -- )  chan.make ( chan i32 ) :> c  [ 20 time.sleep  c 7 chan.send ] spawn  c chan.recv none: [ 0 ] some: [ ] match i32.to-str println ;");
    r.unwrap();
    assert_eq!(out, "7\n");
}

#[test]
fn sleepers_wake_in_deadline_order() {
    let (r, out) = run(": main ( -- )  [ 30 time.sleep \"a\" println ] spawn  [ 10 time.sleep \"b\" println ] spawn  50 time.sleep ;");
    r.unwrap();
    assert_eq!(out, "b\na\n");
}

#[test]
fn time_after_times_out_a_slow_sender() {
    let (r, out) = run(": main ( -- )  chan.make ( chan i32 ) :> c  0 10 time.after :> t  [ 50 time.sleep  c 1 chan.send ] spawn  c recv: [ drop \"value\" ] t recv: [ drop \"timeout\" ] alt println ;");
    r.unwrap();
    assert_eq!(out, "timeout\n");
}

#[test]
fn a_sleeper_left_when_main_returns_is_dropped() {
    let (r, out) =
        run(": main ( -- )  [ 30 time.sleep \"later\" println ] spawn  \"first\" println ;");
    r.unwrap();
    assert_eq!(out, "first\n");
}
