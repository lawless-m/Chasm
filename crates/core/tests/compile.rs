use chasm_core::{compile, Options, Source};

fn ok(src: &str) -> chasm_core::Compilation {
    let c = compile(
        &[Source::new("t.chasm", src)],
        &Options {
            prelude: true,
            test_exports: true,
        },
    );
    for d in &c.diagnostics {
        eprintln!("{}", d.render());
    }
    assert!(c.ok(), "expected success");
    assert!(c.wasm.is_some());
    c
}

fn err(src: &str) -> String {
    let c = compile(&[Source::new("t.chasm", src)], &Options::default());
    assert!(!c.ok(), "expected an error");
    c.diagnostics[0].code.clone()
}

#[test]
fn prelude_compiles() {
    ok("");
}

#[test]
fn simple_words() {
    let c = ok(": square ( i32 -- i32 ) dup i32.mul ;\ntest square : 3 square -> 9\n: main ( -- ) \"hi\" println ;");
    assert!(c.has_main);
    assert_eq!(c.tests.len(), 1);
}

#[test]
fn control_flow_and_locals() {
    ok(r#"
: sum-to ( i32 -- i32 )
  :> n
  0 :> acc!
  n [ acc i32.add acc! ] times
  acc ;
: abs ( i32 -- i32 ) dup 0 i32.lt_s [ 0 swap i32.sub ] when ;
: sign ( i32 -- i32 ) dup 0 i32.lt_s [ drop -1 ] [ 0 i32.gt_s ] if ;
: countdown ( i32 -- ) [ dup 0 i32.gt_s ] [ 1 i32.sub ] while drop ;
: hypot ( f64 f64 -- f64 )
  dup f64.mul          ( f64 f64 )
  swap dup f64.mul     ( f64 f64 )
  f64.add f64.sqrt ;
"#);
}

#[test]
fn arrays_and_combinators() {
    ok(r#"
: doubled ( array i32 -- array i32 )  [ 2 i32.mul ] map ;
: sum ( array i32 -- i32 )  0 [ i32.add ] fold ;
: evens ( array i32 -- array i32 )  [ 2 i32.rem_u i32.eqz ] filter ;
: iota ( i32 -- array i32 )
  :> n
  n array.new :> a  ( )
  n [ :> i  a i i i32.at-placeholder ] times a ;
"#
    .replace("a i i i32.at-placeholder", "a i i array.at!")
    .as_str());
}

#[test]
fn functions_as_values() {
    ok(r#"
: twice ( i32 [ i32 -- i32 ] -- i32 )  :> f  f call f call ;
: inc ( i32 -- i32 )  1 i32.add ;
test twice : 5 'inc twice -> 7
: k ( -- [ -- i32 ] ) [ 42 ] ;
"#);
}

#[test]
fn errors() {
    assert_eq!(err(": f ( i32 -- i32 ) dup ;"), "E_EFFECT_MISMATCH");
    assert_eq!(err(": f ( -- i32 ) i32.add ;"), "E_STACK_UNDERFLOW");
    assert_eq!(err(": f ( f64 -- i32 ) 1 i32.add ;"), "E_TYPE_MISMATCH");
    assert_eq!(
        err(": f ( i32 -- i32 ) [ 1 ] [ 1 2 ] if ;"),
        "E_BRANCH_MISMATCH"
    );
    assert_eq!(err(": f ( -- ) g ;\n: g ( -- ) ;"), "E_UNDEFINED");
    assert_eq!(err(": f ( -- ) ( i32 ) ;"), "E_ASSERTION");
    assert_eq!(
        err(": f ( i32 -- ) drop ;\n: f ( -- ) ;"),
        "E_REDEFINE_EFFECT"
    );
    assert_eq!(
        err("declare f ( i32 -- )\n: f ( -- ) ;"),
        "E_DECLARE_MISMATCH"
    );
    assert_eq!(err(": f ( -- ) leave ;"), "E_LEAVE");
    assert_eq!(err(": f ( -- ) 5 array.new drop ;"), "E_AMBIGUOUS_TYPE");
    assert_eq!(err(": f ( i32 -- ) :> x 1 x! ;"), "E_LOCAL");
    assert_eq!(err(": f ( i32 -- [ -- i32 ] ) :> x [ x ] ;"), "E_CAPTURE");
    assert_eq!(
        err(": sq ( i32 -- i32 ) dup i32.mul ;\ntest sq : 3 sq -> 9i64"),
        "E_TEST_TYPE"
    );
    assert_eq!(err(": main ( i32 -- ) drop ;"), "E_MAIN_EFFECT");
}

#[test]
fn declare_then_define() {
    let c = ok("declare g ( i32 -- i32 )\n: f ( i32 -- i32 ) g ;\ntest g : 2 g -> 4");
    assert!(c.tests[0].pending);
    assert_eq!(c.unresolved().len(), 1);
    assert_eq!(c.graph.callers("g"), vec!["f".to_string()]);
    let c = ok("declare g ( i32 -- i32 )\n: f ( i32 -- i32 ) g ;\n: g ( i32 -- i32 ) 2 i32.mul ;");
    assert!(c.unresolved().is_empty());
}
