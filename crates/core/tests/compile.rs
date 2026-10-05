use wack_core::{compile, Options, Source};

fn ok(src: &str) -> wack_core::Compilation {
    let c = compile(
        &[Source::new("t.wack", src)],
        &Options {
            prelude: true,
            test_exports: true,
            export: false,
            wasi: false,
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
    let c = compile(&[Source::new("t.wack", src)], &Options::default());
    assert!(!c.ok(), "expected an error");
    c.diagnostics[0].code.clone()
}

#[test]
fn prelude_compiles() {
    let c = ok("");
    let some = c.word("option.some").unwrap();
    assert_eq!(some.effect, "( T -- option T )");
    assert!(some.generic && some.generated && some.library);
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
test twice : 5 [ 1 i32.add ] twice -> 7
test twice : 5 [ ( i32 -- i32 ) 2 i32.mul ] twice -> 20
: ap ( i32 [ i32 -- i32 i32 ] -- i32 i32 ) :> f f call ;
test ap : 3 [ dup ] ap -> 3 3
"#);
    assert_eq!(err(": amb ( -- ) [ dup ] drop ;"), "E_AMBIGUOUS_TYPE");
    assert_eq!(err(": f ( -- ) 3 [ ( i32 -- ) drop ] times ;"), "E_SYNTAX");
}

#[test]
fn closures_capture_immutable_locals() {
    ok(r#"
: adder ( i32 -- [ i32 -- i32 ] ) :> k [ k i32.add ] ;
test adder : 3 adder 4 swap call -> 7
: greet ( str -- [ str -- str ] ) :> g [ g swap str.concat ] ;
: nest ( i32 -- [ -- [ -- i32 ] ] ) :> k [ [ k ] ] ;
test nest : 5 nest call call -> 5
: scale ( vec i32 i32 -- ) :> k [ k i32.mul drop ] vec.each ;
struct cell  v: i32
: counter ( -- [ -- i32 ] ) 0 cell.new :> c [ c c cell.v 1 i32.add cell.v! c cell.v ] ;
"#);
    assert_eq!(err(": f ( i32 -- [ -- i32 ] ) :> n! [ n ] ;"), "E_CAPTURE");
    assert_eq!(err(": f ( i32 -- [ -- ] ) :> n! [ 1 n! ] ;"), "E_CAPTURE");
    assert_eq!(err(": f ( i32 -- [ -- ] ) :> n [ 1 n! ] ;"), "E_LOCAL");
    // Misuse of a closure is a type error, not a capture error.
    assert_eq!(
        err(": adder ( i32 -- [ i32 -- i32 ] ) :> k [ k i32.add ] ;\ntest adder : 1 adder call -> 1"),
        "E_STACK_UNDERFLOW"
    );
}

#[test]
fn function_values_in_fields_and_arrays() {
    ok(r#"
struct op  f: [ i32 -- i32 ]
: run-op ( op i32 -- i32 ) swap op.f call ;
test run-op : [ 3 i32.add ] op.new 4 run-op -> 7
: same ( -- i32 ) [ 1 i32.add ] :> f f op.new f op.new eq ;
test same : same -> 1
: differ ( -- i32 ) [ 1 i32.add ] op.new [ 1 i32.add ] op.new eq ;
test differ : differ -> 0
: table ( -- array [ i32 -- i32 ] ) 2 array.new ( array [ i32 -- i32 ] ) :> a  a 0 [ 1 i32.add ] array.at!  a 1 [ 2 i32.mul ] array.at!  a ;
test table : 5 table 1 array.at call -> 10
: mapped ( -- i32 ) table [ :> f [ f call 1 i32.add ] ] map 1 array.at 5 swap call ;
test mapped : mapped -> 11
union act  | go  f: [ -- i32 ]  | stop
"#);
}

#[test]
fn ring_submit_is_raw() {
    ok("raw : ping ( -- i32 ) 0 0 0 0 ring.submit ;");
    assert_eq!(err(": ping ( -- i32 ) 0 0 0 0 ring.submit ;"), "E_RAW");
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
    assert_eq!(err(": f ( i32 -- [ -- i32 ] ) :> x! [ x ] ;"), "E_CAPTURE");
    assert_eq!(
        err(": sq ( i32 -- i32 ) dup i32.mul ;\ntest sq : 3 sq -> 9 i64"),
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

#[test]
fn step_module_imports_memory_and_table() {
    use wack_core::ast::{Node, NodeKind};
    use wack_core::check::{compile_body, Ctx, Mode, Origin, Word, WordKind};
    use wack_core::module::{assemble_step, export_name};
    use wack_core::types::{Effect, Ty};
    use wack_core::Location;

    let mut ctx = Ctx::default();
    ctx.indirect_calls = true;
    ctx.begin_literals(0x30_0000);
    let e = Effect::new(vec![Ty::I32], vec![Ty::I32]);
    let name = |n: &str| Node {
        kind: NodeKind::Name(n.into()),
        loc: Location::default(),
    };
    for (w, body) in [
        ("f", vec![name("dup"), name("i32.mul")]),
        ("g", vec![name("f")]),
    ] {
        let id = ctx.add_word(Word {
            name: w.into(),
            effect: e.clone(),
            body: None,
            failed: false,
            export: false,
            raw: false,
            origin: Origin::User,
            kind: WordKind::Named,
            loc: Location::default(),
            callees: vec![],
            inferred: false,
            generic: None,
            instance_of: None,
            generated: None,
        });
        let out = compile_body(
            &mut ctx,
            w,
            Mode::Declared(&e),
            &body,
            &Location::default(),
            &[],
        )
        .unwrap_or_else(|d| panic!("{}", d.render()));
        ctx.words[id].body = Some(out.compiled);
    }

    let bytes = assemble_step(&mut ctx, &[0, 1], false);
    wack_core::validate(&bytes).unwrap();
    let mut imports = Vec::new();
    let mut exports = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            wasmparser::Payload::ImportSection(r) => {
                for i in r.into_imports() {
                    let i = i.unwrap();
                    imports.push((i.module.to_string(), i.name.to_string()));
                }
            }
            wasmparser::Payload::ExportSection(r) => {
                for e in r {
                    exports.push(e.unwrap().name.to_string());
                }
            }
            _ => {}
        }
    }
    let wack = |n: &str| ("wack".to_string(), n.to_string());
    assert_eq!(
        imports,
        vec![wack("ring_enter"), wack("memory"), wack("table")]
    );
    assert_eq!(exports, vec![export_name(0), export_name(1)]);
    assert_eq!(exports, vec!["w0", "w1"]);

    let shared = assemble_step(&mut ctx, &[0, 1], true);
    wack_core::validate(&shared).unwrap();
}

#[test]
fn unknown_struct_names() {
    for src in [
        ": f ( foo -- ) drop ;",
        ": f ( i32 -- i32 ) ( a b ) ;",
        "declare g ( -- array foo )",
        ": f ( [ foo -- ] -- ) drop ;",
    ] {
        assert_eq!(err(src), "E_UNKNOWN_TYPE", "{src}");
    }
}

const POINT: &str = "struct point  x: i32  y: f64\n";

#[test]
fn struct_generated_words() {
    let c = ok(&format!(
        "{POINT}: origin ( -- point ) 0 0.0 point.new ;\ntest origin : origin point.x -> 0"
    ));
    assert_eq!(c.word("point.new").unwrap().effect, "( i32 f64 -- point )");
    assert_eq!(c.word("point.x!").unwrap().effect, "( point i32 -- )");
    for w in ["point.new", "point.x", "point.x!", "point.y", "point.y!"] {
        assert!(c.word(w).is_some(), "{w}");
    }
    let twice = ok(&format!("{POINT}{POINT}"));
    assert!(twice.diagnostics.is_empty());
    assert_eq!(
        err(&format!("{POINT}struct point  x: i32")),
        "E_REDEFINE_EFFECT"
    );
    let c = ok(&format!("{POINT}struct seg  a: point  b: point"));
    assert_eq!(c.word("seg.a").unwrap().effect, "( seg -- point )");
    ok("struct node  next: node  v: i32");
    assert_eq!(
        ok("struct t  s: str").word("t.s").unwrap().effect,
        "( t -- str )"
    );
    let c = ok(&format!("{POINT}struct bag  items: array point"));
    assert_eq!(
        c.word("bag.items").unwrap().effect,
        "( bag -- array point )"
    );
    assert_eq!(err("struct a  b: b\nstruct b  x: i32"), "E_UNKNOWN_TYPE");
    assert_eq!(
        err(&format!("{POINT}test point.new : 1 2.0 point.new -> 1")),
        "E_TEST_TYPE"
    );
}

#[test]
fn struct_values_move_like_any_value() {
    ok(&format!(
        "{POINT}: swap-xy ( point -- point )  :> p  p point.y i32.trunc_f64_s  p point.x f64.convert_i32_s  point.new ;\n\
         : last ( point point -- point )  :> b :> a!  b a!  a ;\n\
         : pick ( i32 point point -- point )  :> b :> a  [ a ] [ b ] if ;\n\
         : bump ( point i32 -- point )  [ drop dup dup point.x 1 i32.add point.x! ] times ;\n\
         : ap ( point [ point -- i32 ] -- i32 )  call ;\n\
         : id ( point -- point ) ;\n\
         : mixed ( point f64 point -- point f64 point )  rot rot rot ;"
    ));
}

#[test]
fn struct_array_views() {
    ok(&format!(
        "{POINT}: pts ( i32 -- array point )  :> n  n array.new ( array point ) :> a  n [ :> i  a i  i i f64.convert_i32_s point.new  array.at! ] times  a ;\n\
         : mid ( array point -- array point )  1 2 array.slice ;\n\
         : first-x ( array point -- i32 )  0 array.at point.x ;\n\
         struct bag  items: array point  n: i32\n\
         : count ( bag -- i32 )  bag.items array.len ;"
    ));
}

#[test]
fn struct_array_combinators() {
    ok(&format!(
        "{POINT}: sum-x ( array point -- i32 )  0 [ point.x i32.add ] fold ;\n\
         : xs ( array point -- array i32 )  [ point.x ] map ;\n\
         : to-points ( array i32 -- array point )  [ :> n  n n f64.convert_i32_s point.new ] map ;\n\
         : doubled ( array point -- array point )  [ :> p  p point.x 2 i32.mul  p point.y  point.new ] map ;\n\
         : big ( array point -- array point )  [ point.x 1 i32.gt_s ] filter ;\n\
         : count ( array point -- i32 )  0 :> n!  [ drop n 1 i32.add n! ] each  n ;\n\
         : furthest ( array point -- point )  0 0.0 point.new [ :> p :> best  p point.x best point.x i32.gt_s [ p ] [ best ] if ] fold ;"
    ));
}

#[test]
fn dead_words() {
    let c = ok("struct point  x: i32  y: f64
: helper ( -- i32 ) 1 ;
: unused ( -- i32 ) 2 ;
: only-tested ( -- i32 ) 3 ;
test only-tested : only-tested -> 3
: ticked ( -- i32 ) 4 ;
: in-quote ( -- ) ;
: main ( -- ) helper drop 'ticked drop 1 [ drop in-quote ] times ;");
    let dead: Vec<&str> = c.dead().unwrap().iter().map(|w| w.name.as_str()).collect();
    assert_eq!(dead, ["unused", "only-tested"]);
    let lib = ok(": a ( -- i32 ) 1 ;");
    assert!(lib.dead().is_none(), "no roots, nothing to report");
    let exported = ok(": b ( -- i32 ) 1 ;\nexport : a ( -- i32 ) b ;\n: c ( -- ) ;");
    let dead: Vec<&str> = exported
        .dead()
        .unwrap()
        .iter()
        .map(|w| w.name.as_str())
        .collect();
    assert_eq!(dead, ["c"]);
}

fn export(src: &str) -> wack_core::Compilation {
    compile(
        &[Source::new("t.wack", src)],
        &Options {
            prelude: true,
            test_exports: false,
            export: true,
            wasi: false,
        },
    )
}

/// The number of struct and array types in a module's type section.
fn gc_types(wasm: &[u8]) -> usize {
    let mut n = 0;
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        if let wasmparser::Payload::TypeSection(r) = payload.unwrap() {
            for group in r {
                for t in group.unwrap().types() {
                    if !matches!(
                        t.composite_type.inner,
                        wasmparser::CompositeInnerType::Func(_)
                    ) {
                        n += 1;
                    }
                }
            }
        }
    }
    n
}

#[test]
fn closures_cost_nothing_unless_used() {
    let sieve = ": main ( -- ) 10 array.new ( array i32 ) [ drop ] each ;";
    let c = export(sieve);
    assert!(c.ok(), "{:?}", c.diagnostics);
    assert_eq!(gc_types(c.wasm.as_ref().unwrap()), 0);
    let c = export(": inc ( i32 -- i32 ) 1 i32.add ;\n: main ( -- ) 1 'inc call drop ;");
    assert!(c.ok(), "{:?}", c.diagnostics);
    // `$closure` and its GC array.
    assert_eq!(gc_types(c.wasm.as_ref().unwrap()), 2);
}

#[test]
fn channels_cost_nothing_unless_used() {
    let c = export(": main ( -- ) 10 array.new ( array i32 ) [ drop ] each ;");
    assert!(c.ok(), "{:?}", c.diagnostics);
    let wasm = c.wasm.as_ref().unwrap();
    assert_eq!(gc_types(wasm), 0);
    assert!(!functions(wasm).iter().any(|f| f.starts_with("chan.")));
    assert_eq!(imports_and_exports(wasm).0, ["wack.ring_enter"]);
}

#[test]
fn an_unwind_module_imports_frames_and_holds_one_frame_type() {
    use wack_core::check::{Ctx, Unwind};
    use wack_core::module::{assemble, ModuleOptions};
    use wasm_encoder::ValType;
    let mut ctx = Ctx::default();
    ctx.unwind = Unwind::All;
    let frame = ctx.frame_type();
    assert_eq!(ctx.frame_type(), frame);
    let site = ctx.site_frame_type(vec![ValType::I32, wack_core::types::ref_ty(frame)]);
    assert_eq!(
        ctx.site_frame_type(vec![ValType::I32, wack_core::types::ref_ty(frame)]),
        site
    );
    ctx.site_frame_type(vec![ValType::I32]);
    let wasm = assemble(
        &mut ctx,
        &ModuleOptions {
            test_exports: Vec::new(),
            live: None,
            wasi: false,
        },
    );
    wack_core::validate(&wasm).unwrap();
    assert_eq!(
        imports_and_exports(&wasm).0,
        ["wack.ring_enter", "wack.frames", "wack.spawn"]
    );
    let mut non_final = 0;
    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        if let wasmparser::Payload::TypeSection(r) = payload.unwrap() {
            for group in r {
                for t in group.unwrap().types() {
                    if !t.is_final {
                        non_final += 1;
                    }
                }
            }
        }
    }
    assert_eq!(non_final, 1, "$frame, once");
    assert_eq!(gc_types(&wasm), 3, "$frame and two site frames");
}

fn unwind_all(path: &str, src: &str) -> wack_core::Compilation {
    let c = wack_core::program::compile_with_unwind(
        &[Source::new(path, src)],
        &Options {
            prelude: true,
            test_exports: true,
            export: false,
            wasi: false,
        },
        wack_core::check::Unwind::All,
    );
    for d in &c.diagnostics {
        eprintln!("{}", d.render());
    }
    assert!(c.ok(), "{path} compiles with Unwind::All");
    c
}

#[test]
fn a_transformed_process_program_validates() {
    let c = unwind_all(
        "t.wack",
        r#"
: f ( chan i32 -- i32 ) chan.recv none: [ 0 ] some: [ ] match ;
: main ( -- )
  chan.make ( chan i32 ) :> c
  [ c 7 chan.send ] spawn
  c f i32.to-str println ;
"#,
    );
    let wasm = c.wasm.as_ref().unwrap();
    wack_core::validate(wasm).unwrap();
    let fs = functions(wasm);
    assert!(fs.iter().any(|f| f == "f"), "{fs:?}");
    assert!(fs.iter().any(|f| f == "chan.recv<i32>"), "{fs:?}");
}

#[test]
fn transformed_words_skip_runs_and_validate() {
    let c = unwind_all(
        "t.wack",
        r#"
: g ( chan i32 i32 -- i32 )
  :> n :> c
  0 :> acc!
  n [
    :> i
    c chan.recv none: [ 0 ] some: [ ] match
    acc i32.add i i32.add acc!
    acc 10 i32.gt_s [ leave ] when
  ] times
  acc 0 i32.lt_s [ "negative" trap ] when
  1 2 swap drop acc i32.add ;
: main ( -- )
  chan.make ( chan i32 ) :> c
  [ c 7 chan.send  c chan.close ] spawn
  c 3 g i32.to-str println ;
"#,
    );
    let wasm = c.wasm.as_ref().unwrap();
    wack_core::validate(wasm).unwrap();
}

#[test]
fn a_receive_inside_each_construct_validates_transformed() {
    let recv = "c chan.recv none: [ 0 ] some: [ ] match";
    let cases = [
        ("if then", ": t ( chan i32 i32 -- i32 ) :> x :> c  x [ R ] [ 1 ] if ;"),
        ("if else", ": t ( chan i32 i32 -- i32 ) :> x :> c  x [ 1 ] [ R ] if ;"),
        ("when", ": t ( chan i32 i32 -- i32 ) :> x :> c  5 x [ drop R ] when ;"),
        ("unless", ": t ( chan i32 i32 -- i32 ) :> x :> c  5 x [ drop R ] unless ;"),
        (
            "while body",
            ": t ( chan i32 -- i32 ) :> c  0 :> n!  [ n 3 i32.lt_s ] [ R n i32.add 1 i32.add n! ] while  n ;",
        ),
        (
            "while condition",
            ": t ( chan i32 -- i32 ) :> c  0 :> n!  [ R 0 i32.ne ] [ n 1 i32.add n! ] while  n ;",
        ),
        (
            "until body",
            ": t ( chan i32 -- i32 ) :> c  0 :> n!  [ R n i32.add 1 i32.add n! ] [ n 3 i32.ge_s ] until  n ;",
        ),
        ("times", ": t ( chan i32 -- i32 ) :> c  0 3 [ drop R i32.add ] times ;"),
        (
            "each",
            ": t ( chan i32 -- i32 ) :> c  0 :> n!  3 array.new ( array i32 ) [ R i32.add n i32.add n! ] each  n ;",
        ),
        (
            "map",
            ": t ( chan i32 -- i32 ) :> c  3 array.new ( array i32 ) [ R i32.add ] map  array.len ;",
        ),
        (
            "filter",
            ": t ( chan i32 -- i32 ) :> c  3 array.new ( array i32 ) [ drop R 0 i32.ne ] filter  array.len ;",
        ),
        (
            "fold",
            ": t ( chan i32 -- i32 ) :> c  3 array.new ( array i32 ) 0 [ i32.add R i32.add ] fold ;",
        ),
    ];
    for (what, src) in cases {
        let src = src.replace('R', recv);
        let c = unwind_all(what, &src);
        if let Err(e) = wack_core::validate(c.wasm.as_ref().unwrap()) {
            panic!("{what}: {e}");
        }
    }
}

#[test]
fn a_receive_inside_match_and_alt_arms_validates_transformed() {
    let recv = "c chan.recv none: [ 0 ] some: [ ] match";
    let cases = [
        (
            "some arm",
            ": t ( chan i32 option i32 -- i32 ) :> o :> c  o none: [ 0 ] some: [ drop R ] match ;",
        ),
        (
            "none arm",
            ": t ( chan i32 option i32 -- i32 ) :> o :> c  o none: [ R ] some: [ ] match ;",
        ),
        (
            "else arm",
            ": t ( chan i32 option i32 -- i32 ) :> o :> c  o none: [ 0 ] else: [ drop R ] match ;",
        ),
        (
            "second alt arm",
            ": t ( chan i32 chan i32 -- i32 ) :> b :> c \
             c recv: [ none: [ 0 ] some: [ ] match ] b recv: [ drop R ] alt ;",
        ),
        (
            "alt in a while in an if",
            ": t ( chan i32 chan i32 i32 -- i32 ) :> x :> b :> c  0 :> n! \
             x [ [ n 3 i32.lt_s ] \
                 [ c recv: [ drop n 1 i32.add n! ] b recv: [ drop R n i32.add n! ] alt ] \
                 while ] [ ] if  n ;",
        ),
    ];
    for (what, src) in cases {
        let src = src.replace('R', recv);
        let c = unwind_all(what, &src);
        if let Err(e) = wack_core::validate(c.wasm.as_ref().unwrap()) {
            panic!("{what}: {e}");
        }
    }
}

/// Whether the function named `name` reads the unwind mode cell (it is
/// transformed).
fn reads_unwind_mode(wasm: &[u8], name: &str) -> bool {
    let names = functions(wasm);
    let index = names.iter().position(|n| n == name).expect(name) as u32;
    let mut imports = 0;
    let mut k = 0;
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        match payload.unwrap() {
            wasmparser::Payload::ImportSection(r) => {
                imports = r
                    .into_imports()
                    .filter(|i| matches!(i.as_ref().unwrap().ty, wasmparser::TypeRef::Func(_)))
                    .count() as u32;
            }
            wasmparser::Payload::CodeSectionEntry(body) => {
                if imports + k == index {
                    let mut ops = body.get_operators_reader().unwrap();
                    while !ops.eof() {
                        if let wasmparser::Operator::I32Load { memarg } = ops.read().unwrap() {
                            if memarg.offset == wack_core::layout::UNWIND_MODE as u64 {
                                return true;
                            }
                        }
                    }
                    return false;
                }
                k += 1;
            }
            _ => {}
        }
    }
    panic!("no body for {name}")
}

#[test]
fn process_programs_are_transformed() {
    let c = export(
        r#"
: add1 ( i32 -- i32 ) 1 i32.add ;
: f ( chan i32 -- i32 ) chan.recv none: [ 0 ] some: [ ] match ;
: main ( -- )
  chan.make ( chan i32 ) :> c
  [ c 7 chan.send ] spawn
  c f add1 i32.to-str println ;
"#,
    );
    assert!(c.ok(), "{:?}", c.diagnostics);
    assert_eq!(c.processes.as_deref(), Some("main"));
    let wasm = c.wasm.as_ref().unwrap();
    wack_core::validate(wasm).unwrap();
    assert_eq!(
        imports_and_exports(wasm).0,
        ["wack.ring_enter", "wack.frames", "wack.spawn"]
    );
    assert!(reads_unwind_mode(wasm, "main"));
    assert!(reads_unwind_mode(wasm, "f"));
    assert!(reads_unwind_mode(wasm, "chan.recv<i32>"));
    assert!(reads_unwind_mode(wasm, "rt.ring"));
    assert!(!reads_unwind_mode(wasm, "add1"), "never on a waiting path");
}

#[test]
fn programs_without_processes_are_unchanged() {
    let c = export(": add1 ( i32 -- i32 ) 1 i32.add ;\n: main ( -- ) 1 add1 i32.to-str println ;");
    assert!(c.ok(), "{:?}", c.diagnostics);
    let wasm = c.wasm.as_ref().unwrap();
    assert_eq!(imports_and_exports(wasm).0, ["wack.ring_enter"]);
    assert_eq!(gc_types(wasm), 0);
    assert!(!reads_unwind_mode(wasm, "main"));
}

#[test]
fn every_example_validates_transformed() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["examples"] {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "wack") {
                files.push(p);
            }
        }
    }
    files.sort();
    assert!(files.len() > 10);
    for p in files {
        let src = std::fs::read_to_string(&p).unwrap();
        let c = unwind_all(&p.display().to_string(), &src);
        if let Err(e) = wack_core::validate(c.wasm.as_ref().unwrap()) {
            panic!("{}: {e}", p.display());
        }
    }
}

#[test]
fn spawn_imports_the_global_only_when_used() {
    let c = export(": main ( -- ) [ \"hi\" println ] spawn ;");
    assert!(c.ok(), "{:?}", c.diagnostics);
    let wasm = c.wasm.as_ref().unwrap();
    wack_core::validate(wasm).unwrap();
    // A program using processes is transformed: `wack.frames` comes first.
    assert_eq!(
        imports_and_exports(wasm).0,
        ["wack.ring_enter", "wack.frames", "wack.spawn"]
    );
    let c = export(": main ( -- ) 10 array.new ( array i32 ) [ drop ] each ;");
    assert_eq!(
        imports_and_exports(c.wasm.as_ref().unwrap()).0,
        ["wack.ring_enter"]
    );
    assert_eq!(err(": main ( -- ) [ 1 ] spawn ;"), "E_TYPE_MISMATCH");
}

#[test]
fn alt_over_two_channels() {
    let pick = ": pick ( chan i32 chan str -- i32 ) :> b :> a  a recv: [ none: [ -1 ] some: [ ] match ] b recv: [ none: [ -2 ] some: [ str.len ] match ] alt ;";
    let c = export(&format!(
        "{pick}\n: main ( -- ) chan.make ( chan i32 ) :> a  chan.make ( chan str ) :> b  a b pick i32.to-str println ;"
    ));
    assert!(c.ok(), "{:?}", c.diagnostics);
    let wasm = c.wasm.as_ref().unwrap();
    wack_core::validate(wasm).unwrap();
    let fs = functions(wasm);
    assert!(fs.iter().any(|f| f == "chan.taken<i32>"), "{fs:?}");
    assert!(fs.iter().any(|f| f == "chan.taken<str>"), "{fs:?}");
    ok(pick);
    assert_eq!(
        err(": f ( chan i32 chan str -- i32 ) :> b :> a  a recv: [ drop 1 ] b recv: [ drop \"x\" ] alt ;"),
        "E_BRANCH_MISMATCH"
    );
    assert_eq!(err(": f ( -- ) 1 recv: [ drop ] alt ;"), "E_TYPE_MISMATCH");
}

#[test]
fn processes_names_the_first_offending_word() {
    let c = export(": main ( -- ) [ \"hi\" println ] spawn ;");
    assert_eq!(c.processes.as_deref(), Some("main"));
    let c = export(": main ( -- ) chan.make ( chan i32 ) chan.recv drop ;");
    assert!(
        c.processes
            .as_deref()
            .is_some_and(|w| w.starts_with("chan.")),
        "{:?}",
        c.processes
    );
    let dead = ": unused ( -- ) chan.make ( chan i32 ) chan.recv drop ;\n: main ( -- ) 1 drop ;";
    assert_eq!(export(dead).processes, None);
    assert!(ok(dead).processes.is_some(), "no roots: every word counts");
    assert_eq!(
        export(": main ( -- ) 10 array.new ( array i32 ) [ drop ] each ;").processes,
        None
    );
    assert_eq!(ok(": f ( -- ) [ ] spawn ;").processes.as_deref(), Some("f"));
}

#[test]
fn prelude_chan() {
    ok(": f ( -- chan i32 ) chan.make ( chan i32 ) ;");
    let src = r#"
: show ( option i32 -- )  none: [ "done" println ] some: [ i32.to-str println ] match ;
: main ( -- )
  chan.make ( chan i32 ) :> c
  c chan.sender
  c 1 chan.send
  c chan.recv show
  c chan.close c chan.close ;
"#;
    let c = export(src);
    assert!(c.ok(), "{:?}", c.diagnostics);
    wack_core::validate(c.wasm.as_ref().unwrap()).unwrap();
    assert!(functions(c.wasm.as_ref().unwrap())
        .iter()
        .any(|f| f == "chan.recv<i32>"));
}

fn functions(wasm: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        if let Ok(wasmparser::Payload::CustomSection(c)) = payload {
            if let wasmparser::KnownCustom::Name(reader) = c.as_known() {
                for sub in reader.into_iter().flatten() {
                    if let wasmparser::Name::Function(map) = sub {
                        names.extend(map.into_iter().flatten().map(|n| n.name.to_string()));
                    }
                }
            }
        }
    }
    names
}

#[test]
fn export_refuses_reachable_unresolved_words() {
    let c = export("declare later ( -- i32 )\n: main ( -- ) later drop ;");
    assert!(!c.ok());
    let d = &c.diagnostics[0];
    assert_eq!(d.code, "E_UNRESOLVED");
    assert_eq!(d.dependants, Some(vec!["main".to_string()]));
    let c = export("declare never ( -- i32 )\n: main ( -- ) ;");
    assert!(c.ok(), "an unreachable stub is dropped, not refused");
    let c = export("declare stub ( -- i32 )\n: lib ( -- i32 ) stub ;");
    assert!(c.ok(), "with no root nothing is refused");
}

#[test]
fn export_leaves_out_dead_words() {
    let src = ": helper ( -- i32 ) 1 ;\n: unused ( -- i32 ) 2 ;\n: ticked ( -- i32 ) 4 ;\n: main ( -- ) helper drop 'ticked call drop ;";
    let full = ok(src);
    let trimmed = export(src);
    assert!(trimmed.ok());
    let names = functions(trimmed.wasm.as_ref().unwrap());
    for live in ["helper", "ticked", "main"] {
        assert!(names.iter().any(|n| n == live), "{live} kept: {names:?}");
    }
    assert!(!names.iter().any(|n| n == "unused"), "{names:?}");
    assert!(trimmed.wasm.unwrap().len() < full.wasm.unwrap().len());
}

fn imports_and_exports(wasm: &[u8]) -> (Vec<String>, Vec<String>) {
    let (mut imports, mut exports) = (Vec::new(), Vec::new());
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        match payload.unwrap() {
            wasmparser::Payload::ImportSection(r) => {
                for i in r.into_imports() {
                    let i = i.unwrap();
                    imports.push(format!("{}.{}", i.module, i.name));
                }
            }
            wasmparser::Payload::ExportSection(r) => {
                exports.extend(r.into_iter().map(|e| e.unwrap().name.to_string()));
            }
            _ => {}
        }
    }
    (imports, exports)
}

fn wasi(src: &str) -> wack_core::Compilation {
    compile(
        &[Source::new("t.wack", src)],
        &Options {
            prelude: true,
            test_exports: false,
            export: true,
            wasi: true,
        },
    )
}

#[test]
fn wasi_build_refuses_processes() {
    let src = ": main ( -- ) [ ] spawn ;";
    let c = wasi(src);
    assert_eq!(
        c.diagnostics[0].code, "E_WASI_UNSUPPORTED",
        "{:?}",
        c.diagnostics
    );
    assert!(c.wasm.is_none());
    assert!(export(src).ok());
    // Channels and spawning only in dead words: nothing to refuse, and no
    // `wack.spawn` import.
    let c = wasi(": unused ( -- ) chan.make ( chan i32 ) :> c c 1 chan.send [ ] spawn ;\n: main ( -- ) \"hi\" println ;");
    assert!(c.ok(), "{:?}", c.diagnostics);
    assert!(!imports_and_exports(c.wasm.as_ref().unwrap())
        .0
        .iter()
        .any(|i| i == "wack.spawn"));
}

#[test]
fn wasi_build_imports_preview1_and_exports_start() {
    let src = ": main ( -- ) \"hi\" println ;";
    let c = compile(
        &[Source::new("t.wack", src)],
        &Options {
            prelude: true,
            test_exports: false,
            export: true,
            wasi: true,
        },
    );
    assert!(c.ok(), "{:?}", c.diagnostics);
    let (imports, exports) = imports_and_exports(c.wasm.as_ref().unwrap());
    assert_eq!(
        imports,
        [
            "wasi_snapshot_preview1.fd_write",
            "wasi_snapshot_preview1.fd_read",
            "wasi_snapshot_preview1.fd_close",
            "wasi_snapshot_preview1.path_open",
            "wasi_snapshot_preview1.clock_time_get",
        ]
    );
    assert!(exports.iter().any(|e| e == "_start"), "{exports:?}");
    assert!(exports.iter().any(|e| e == "memory"), "{exports:?}");
    let plain = export(src);
    let (imports, exports) = imports_and_exports(plain.wasm.as_ref().unwrap());
    assert_eq!(imports, ["wack.ring_enter"]);
    assert!(!exports.iter().any(|e| e == "_start"));
}

#[test]
fn declared_generic_words() {
    let c = ok(": twice ( T -- T T ) dup ;");
    let w = c.word("twice").unwrap();
    assert!(w.generic && w.resolved);
    ok(": swap-pair ( T U -- U T ) swap ;");
    ok(": first ( array T -- T ) 0 array.at ;");
    assert_eq!(err(": bad ( T T -- T ) i32.add ;"), "E_TYPE_MISMATCH");
    assert_eq!(err(": bad2 ( T -- T ) ( U ) ;"), "E_ASSERTION");
    assert_eq!(err("export : e ( T -- T ) ;"), "E_NEEDS_EFFECT");
    assert_eq!(err("struct box  v: T"), "E_UNKNOWN_TYPE");
    let c = compile(
        &[Source::new("t.wack", "declare g ( array T -- i32 )")],
        &Options::default(),
    );
    let names: Vec<&str> = c.unresolved().iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["g"]);
}

#[test]
fn some_words_must_declare_their_effect() {
    assert_eq!(
        err(": fact dup 1 i32.gt_s [ dup 1 i32.sub fact i32.mul ] when ;"),
        "E_NEEDS_EFFECT"
    );
    let c = compile(
        &[Source::new(
            "t.wack",
            "declare odd? ( i32 -- i32 )\n: even? dup i32.eqz [ drop 1 ] [ 1 i32.sub odd? ] if ;\n: odd? ( i32 -- i32 ) dup i32.eqz [ drop 0 ] [ 1 i32.sub even? ] if ;",
        )],
        &Options::default(),
    );
    let d = &c.diagnostics[0];
    assert_eq!(d.code, "E_NEEDS_EFFECT");
    assert!(
        d.message.contains("`even?`") && d.message.contains("`odd?`"),
        "{}",
        d.message
    );
    assert_eq!(err("export : e 1 ;"), "E_NEEDS_EFFECT");
    assert_eq!(err(": main \"x\" println ;"), "E_NEEDS_EFFECT");
    let c = compile(
        &[Source::new(
            "t.wack",
            ": fact ( i32 -- i32 ) ;\n: f :> fact fact ;",
        )],
        &Options::default(),
    );
    assert!(
        c.diagnostics.iter().all(|d| d.code != "E_NEEDS_EFFECT"),
        "{:?}",
        c.diagnostics
    );
}

#[test]
fn inferred_effects() {
    let effect = |src: &str, word: &str| -> (String, bool, bool) {
        let c = ok(src);
        let w = c.word(word).unwrap();
        (w.effect.clone(), w.inferred, w.generic)
    };
    assert_eq!(
        effect(": sq dup i32.mul ;", "sq"),
        ("( i32 -- i32 )".into(), true, false)
    );
    assert_eq!(
        effect(": add3 i32.add i32.add ;", "add3").0,
        "( i32 i32 i32 -- i32 )"
    );
    assert_eq!(
        effect(": twice dup ;", "twice"),
        ("( T -- T T )".into(), true, true)
    );
    assert_eq!(
        effect(
            ": countdown [ dup 0 i32.gt_s ] [ 1 i32.sub ] while drop ;",
            "countdown"
        )
        .0,
        "( i32 -- )"
    );
    assert_eq!(effect(": hello \"hi\" println ;", "hello").0, "( -- )");
    assert_eq!(effect(": die \"x\" trap ;", "die").0, "( -- )");
    assert_eq!(
        effect(": mk 3 array.new ( array i32 ) ;", "mk").0,
        "( -- array i32 )"
    );
    assert_eq!(err(": bad 1 \"x\" i32.add ;"), "E_TYPE_MISMATCH");
    assert_eq!(err(": f 1 ;\n: f 2 i64 ;"), "E_REDEFINE_EFFECT");
    assert_eq!(
        err("declare g ( i32 -- i32 )\n: g 1 i64 ;"),
        "E_DECLARE_MISMATCH"
    );
    assert_eq!(err(": g [ dup ] when ;"), "E_BRANCH_MISMATCH");
    let c = ok(": sq dup i32.mul ;\n: main ( -- ) 3 sq i32.to-str println ;");
    assert!(c.wasm.is_some());
    assert!(!c.word("main").unwrap().inferred);
}

#[test]
fn generic_instances() {
    let c = ok(": twice ( T -- T T ) dup ;\n: a ( i32 -- i32 i32 ) twice ;\n: b ( str -- str str ) twice ;");
    let inst = |name: &str| c.words.iter().find(|w| w.name == name).cloned();
    assert_eq!(
        inst("twice<i32>").unwrap().instance_of.as_deref(),
        Some("twice")
    );
    assert_eq!(inst("twice<str>").unwrap().effect, "( str -- str str )");
    let callees: Vec<String> = c
        .graph
        .callees("a")
        .iter()
        .map(|e| e.word.clone())
        .collect();
    assert_eq!(callees, ["twice<i32>"]);
    assert_eq!(c.graph.callers("twice"), ["twice<i32>", "twice<str>"]);
    ok(": first ( array T -- T ) 0 array.at ;\n: f ( array f64 -- f64 ) first ;");
    let c = ok(": twice ( T -- T T ) dup ;\n: thrice ( T -- T T T ) twice twice ;\n: q ( i32 -- i32 i32 i32 ) thrice ;");
    assert!(c.words.iter().any(|w| w.name == "thrice<i32>"));
    assert!(c.words.iter().any(|w| w.name == "twice<i32>"));
    ok(": twice ( T -- T T ) dup ;\n: amb ( -- i32 ) 1 twice drop ;");
    assert_eq!(
        err(": twice ( T -- T T ) dup ;\n: amb2 ( -- ) 0 array.new twice 2drop ;"),
        "E_AMBIGUOUS_TYPE"
    );
    ok(": twice ( T -- T T ) dup ;\ntest twice : 3 twice -> 3 3");
}

#[test]
fn whole_program_emits_only_reachable_instances() {
    let src = ": twice ( T -- T T ) dup ;\n: unused ( str -- str str ) twice ;\n: main ( -- ) 3 twice i32.add drop ;";
    let c = export(src);
    assert!(c.ok(), "{:?}", c.diagnostics);
    let names = functions(c.wasm.as_ref().unwrap());
    assert!(names.iter().any(|n| n == "twice<i32>"), "{names:?}");
    assert!(
        !names.iter().any(|n| n == "twice" || n == "twice<str>"),
        "{names:?}"
    );
    let dead: Vec<String> = c.dead().unwrap().iter().map(|w| w.name.clone()).collect();
    assert!(
        dead.contains(&"unused".to_string()) && dead.contains(&"twice<str>".to_string()),
        "{dead:?}"
    );
    assert!(!dead.contains(&"twice".to_string()), "{dead:?}");
    let c = export("declare g ( T -- T )\n: main ( -- ) 1 g drop ;");
    let unresolved: Vec<&wack_core::Diagnostic> = c
        .diagnostics
        .iter()
        .filter(|d| d.code == "E_UNRESOLVED")
        .collect();
    assert_eq!(unresolved.len(), 1, "{:?}", c.diagnostics);
    assert_eq!(unresolved[0].word.as_deref(), Some("g"));
}

#[test]
fn redefining_a_generic_rebuilds_instances() {
    let c = ok(": twice ( T -- T T ) dup ;\n: a ( i32 -- i32 i32 ) twice ;\n: twice ( T -- T T ) dup drop dup ;");
    assert_eq!(c.words.iter().filter(|w| w.name == "twice<i32>").count(), 1);
}

#[test]
fn ticking_a_generic_word() {
    use wack_core::graph::EdgeKind;
    let g = ": twice ( T -- T T ) dup ;\n";
    let c = ok(&format!("{g}: t1 ( -- [ i32 -- i32 i32 ] ) 'twice ;"));
    let callees: Vec<(String, EdgeKind)> = c
        .graph
        .callees("t1")
        .iter()
        .map(|e| (e.word.clone(), e.kind))
        .collect();
    assert_eq!(
        callees,
        [
            ("[tick twice<i32>]".to_string(), EdgeKind::AddressTaken),
            ("twice<i32>".to_string(), EdgeKind::AddressTaken)
        ]
    );
    ok(&format!(
        "{g}: t2 ( -- ) 'twice ( [ str -- str str ] ) drop ;"
    ));
    ok(&format!(
        "{g}: apply ( i32 [ i32 -- i32 i32 ] -- i32 i32 ) call ;\n: t3 ( -- i32 i32 ) 3 'twice apply ;"
    ));
    assert_eq!(
        err(&format!("{g}: t4 ( -- ) 'twice drop ;")),
        "E_AMBIGUOUS_TYPE"
    );
    ok(&format!(
        "{g}: t5 ( -- [ -- i32 i32 ] ) [ 3 'twice call ] ;"
    ));
}

#[test]
fn struct_type_parameters_are_checked() {
    assert_eq!(
        err("struct pair T U  first: T  second: V"),
        "E_UNKNOWN_TYPE"
    );
    assert_eq!(err("struct point  x: T"), "E_UNKNOWN_TYPE");
}

const SHAPE: &str = "union shape\n  | circle  r: f64\n  | rect    w: f64  h: f64\n  | empty\n";

#[test]
fn union_declarations_and_constructors() {
    let c = ok(SHAPE);
    let effect = |w: &str| c.word(w).unwrap().effect.clone();
    assert_eq!(effect("shape.circle"), "( f64 -- shape )");
    assert_eq!(effect("shape.rect"), "( f64 f64 -- shape )");
    assert_eq!(effect("shape.empty"), "( -- shape )");
    ok(&format!(
        "{SHAPE}: pick ( i32 shape shape -- shape ) :> b :> a [ a ] [ b ] if ;\n: hold ( shape -- shape ) :> s s ;\n: both ( -- shape shape ) 1.0 shape.circle dup ;\n: arr ( -- array shape ) 2 array.new ( array shape ) ;"
    ));
    ok(&format!("{SHAPE}{SHAPE}"));
    assert_eq!(
        err(&format!("{SHAPE}union shape | circle  r: f64")),
        "E_REDEFINE_EFFECT"
    );
    assert_eq!(
        err("struct shape  x: i32\nunion shape | a"),
        "E_REDEFINE_EFFECT"
    );
    assert_eq!(err("union u | a  x: foo"), "E_UNKNOWN_TYPE");
    assert_eq!(
        err(&format!(": f ( shape -- ) drop ;\n{SHAPE}")),
        "E_UNKNOWN_TYPE"
    );
    assert_eq!(
        err(&format!("{SHAPE}test shape.empty : shape.empty -> 1")),
        "E_TEST_TYPE"
    );
    ok("union list | nil | cons  head: i32  tail: list");
}

#[test]
fn union_tag_and_readers() {
    let c = ok(&format!("{SHAPE}: user ( -- ) ;"));
    let effect = |w: &str| c.word(w).unwrap().effect.clone();
    assert_eq!(effect("shape.tag"), "( shape -- i32 )");
    assert_eq!(effect("shape.rect.h"), "( shape -- f64 )");
    assert!(!c.words.iter().any(|w| w.name.starts_with("shape.empty.")));
    assert!(c.word("shape.tag").unwrap().generated);
    assert!(!c.word("user").unwrap().generated);
}

#[test]
fn match_is_a_primitive_name() {
    assert_eq!(err(": match ( -- ) ;"), "E_REDEFINE_EFFECT");
}

const AREA: &str = ": area ( shape -- f64 ) circle: [ :> r  r r f64.mul 3.14 f64.mul ] rect: [ f64.mul ] empty: [ 0.0 ] match ;\n";

#[test]
fn match_on_a_union() {
    ok(&format!("{SHAPE}{AREA}"));
    ok(&format!(
        "{SHAPE}: n ( shape -- i32 ) circle: [ drop 1 ] rect: [ 2drop 2 ] empty: [ 3 ] match ;"
    ));
    ok(&format!(
        "{SHAPE}: m ( shape -- f64 ) circle: [ ] rect: [ f64.add ] empty: [ \"no\" trap ] match ;"
    ));
    ok(&format!(
        "{SHAPE}: first-circle ( array shape -- f64 ) :> a 0.0 a array.len [ :> i a i array.at circle: [ f64.add leave ] rect: [ 2drop ] empty: [ ] match ] times ;"
    ));
    let c = compile(
        &[Source::new(
            "t.wack",
            format!("{SHAPE}: f ( shape -- i32 ) circle: [ drop 1 ] rect: [ 2drop 2 ] match ;"),
        )],
        &Options::default(),
    );
    assert_eq!(c.diagnostics[0].code, "E_MATCH_MISSING");
    assert_eq!(c.diagnostics[0].expected, Some(vec!["empty".to_string()]));
    let arms = |a: &str| format!("{SHAPE}: f ( shape -- i32 ) {a} match ;");
    assert_eq!(
        err(&arms(
            "circle: [ drop 1 ] rect: [ 2drop 2 ] empty: [ 3 ] square: [ 4 ]"
        )),
        "E_MATCH_ARM"
    );
    assert_eq!(
        err(&arms(
            "circle: [ drop 1 ] circle: [ drop 1 ] rect: [ 2drop 2 ] empty: [ 3 ]"
        )),
        "E_MATCH_ARM"
    );
    assert_eq!(
        err(&arms("circle: [ drop 1 ] rect: [ 2drop 2.0 ] empty: [ 3 ]")),
        "E_BRANCH_MISMATCH"
    );
    assert_eq!(
        err(": f ( i32 -- i32 ) a: [ 1 ] match ;"),
        "E_TYPE_MISMATCH"
    );
    assert_eq!(
        err(&format!(
            "{SHAPE}: g circle: [ 1 ] rect: [ 2 ] empty: [ 3 ] match ;"
        )),
        "E_AMBIGUOUS_TYPE"
    );
}

#[test]
fn match_else_arm() {
    ok(&format!(
        "{SHAPE}: area2 ( shape -- f64 ) circle: [ :> r r r f64.mul 3.14 f64.mul ] else: [ drop 0.0 ] match ;"
    ));
    ok(&format!(
        "{SHAPE}: tag2 ( shape -- i32 ) else: [ shape.tag ] match ;"
    ));
    ok(&format!(
        "{SHAPE}: mid ( shape -- f64 ) rect: [ f64.mul ] else: [ shape.tag f64.convert_i32_s ] circle: [ 2.0 f64.mul ] match ;"
    ));
    let arms = |a: &str| format!("{SHAPE}: f ( shape -- i32 ) {a} match ;");
    assert_eq!(
        err(&arms(
            "circle: [ drop 1 ] else: [ drop 2 ] else: [ drop 3 ]"
        )),
        "E_MATCH_ARM"
    );
    assert_eq!(
        err(&arms(
            "circle: [ drop 1 ] rect: [ 2drop 2 ] empty: [ 3 ] else: [ drop 4 ]"
        )),
        "E_MATCH_ARM"
    );
    assert_eq!(
        err(&arms("circle: [ drop 1 ] else: [ ]")),
        "E_BRANCH_MISMATCH"
    );
}

const PAIR: &str = "struct pair T U  first: T  second: U\n";

#[test]
fn generic_structs() {
    let c = ok(&format!("{PAIR}: p ( -- pair i32 str ) 3 \"x\" pair.new ;"));
    assert_eq!(
        c.word("pair.new<i32,str>").expect("instance").effect,
        "( i32 str -- pair i32 str )"
    );
    assert!(c.word("pair.new").unwrap().generic);
    ok(&format!("{PAIR}: f ( pair i32 str -- str ) pair.second ;"));
    let c = ok(&format!(
        "{PAIR}: swap-pair ( pair T U -- pair U T ) :> p  p pair.second p pair.first pair.new ;\n: use ( -- pair str i32 ) 3 \"x\" pair.new swap-pair ;"
    ));
    assert!(c.word("swap-pair<i32,str>").is_some());
    ok(&format!(
        "{PAIR}: g ( -- ) 3 \"x\" pair.new :> p  p 4 pair.first! ;"
    ));
    ok(&format!(
        "{PAIR}: a ( -- array pair i32 str ) 2 array.new ( array pair i32 str ) ;"
    ));
    ok("struct box T  v: T\nstruct node  n: i32  next: box node\n: deep ( node -- i32 ) node.next box.v node.n ;");
    assert_eq!(
        err(&format!("{PAIR}struct pair U T  first: U  second: T")),
        "E_REDEFINE_EFFECT"
    );
    assert_eq!(err("struct w T  next: w i32"), "E_UNKNOWN_TYPE");
    let c = export(&format!(
        "{PAIR}: main ( -- ) 3 \"x\" pair.new pair.second println ;"
    ));
    assert!(c.ok(), "{:?}", c.diagnostics);
    let f = functions(c.wasm.as_ref().unwrap());
    assert!(f.iter().any(|n| n == "pair.new<i32,str>"), "{f:?}");
    assert!(!f.iter().any(|n| n == "pair.new"), "{f:?}");
}

const OPTION: &str = "union option T | none | some  v: T\n";
const LIST: &str = "union list T | nil | cons  head: T  tail: list T\n";

#[test]
fn generic_unions() {
    let c = ok(&format!("{OPTION}: s ( -- option i32 ) 3 option.some ;"));
    assert!(c.word("option.some<i32>").is_some());
    ok(&format!("{OPTION}: n ( -- option i32 ) option.none ;"));
    assert_eq!(
        err(&format!("{OPTION}: bad ( -- ) option.none drop ;")),
        "E_AMBIGUOUS_TYPE"
    );
    ok(&format!(
        "{OPTION}: get ( option i32 -- i32 ) none: [ 0 ] some: [ ] match ;"
    ));
    let c = ok(&format!(
        "{OPTION}: or-else ( option T T -- T ) :> d  none: [ d ] some: [ ] match ;\n: use ( -- str ) \"x\" option.some \"y\" or-else ;"
    ));
    assert!(c.word("or-else<str>").is_some());
    let c = ok(&format!(
        "{LIST}: length ( list T -- i32 ) nil: [ 0 ] cons: [ length 1 i32.add nip ] match ;\n: three ( -- list i32 ) 1 2 3 list.nil list.cons list.cons list.cons ;\n: k ( -- i32 ) three length ;"
    ));
    assert!(c.word("length<i32>").is_some());
    assert!(c.word("list.cons<i32>").is_some());
    ok(&format!(
        "{OPTION}struct node  v: i32  next: option node\n: nx ( node -- option node ) node.next ;"
    ));
    ok(&format!(
        "{OPTION}: a ( -- array option i32 ) 2 array.new ( array option i32 ) ;"
    ));
    assert_eq!(
        err(&format!("{OPTION}union option U | none | some  v: U")),
        "E_REDEFINE_EFFECT"
    );
    assert_eq!(err("union w T | a  x: w i32"), "E_UNKNOWN_TYPE");
}

#[test]
fn prelude_option() {
    let c = ok(": f ( -- option i32 ) 3 option.some ;");
    assert!(c.word("option.some<i32>").is_some());
    let c = export(": f ( -- option i32 ) 3 option.some ;\n: main ( -- ) f none: [ 0 ] some: [ ] match i32.to-str println ;");
    assert!(c.ok(), "{:?}", c.diagnostics);
    let f = functions(c.wasm.as_ref().unwrap());
    assert!(f.iter().any(|n| n == "option.some<i32>"), "{f:?}");
    assert!(!f.iter().any(|n| n == "option.some"), "{f:?}");
    let c = export(": f ( -- option i32 ) 3 option.some ;\n: main ( -- ) ;");
    let f = functions(c.wasm.as_ref().unwrap());
    assert!(!f.iter().any(|n| n.starts_with("option.")), "{f:?}");
}

#[test]
fn eq_and_hash_on_numbers() {
    let c = ok(": same ( T T -- i32 ) eq ;\n: a ( -- i32 ) 1 2 same ;\n: b ( -- i32 ) 1.0 2.0 same ;\n: k ( T -- i32 ) hash ;\n: kk ( -- i32 ) 3 i64 k ;");
    assert!(c.word("same<i32>").is_some());
    assert!(c.word("same<f64>").is_some());
    assert!(c.word("k<i64>").is_some());
    let c = ok(": same2 eq ;");
    assert_eq!(c.word("same2").unwrap().effect, "( T T -- i32 )");
    assert_eq!(err(": f ( -- i32 ) 3 \"x\" eq ;"), "E_TYPE_MISMATCH");
    ok(": g ( array T -- i32 ) 0 array.at hash ;");
    ok(": h ( -- i32 ) 1 array.new ( array i32 ) 0 array.at hash ;");
    ok(": s ( -- i32 ) \"a\" \"a\" eq ;");
}

#[test]
fn eq_and_hash_helpers() {
    let c = export("struct point  x: i32  y: f64\n: main ( -- ) 1 2.0 point.new 1 2.0 point.new eq i32.to-str println ;");
    assert!(c.ok(), "{:?}", c.diagnostics);
    let f = functions(c.wasm.as_ref().unwrap());
    assert!(f.iter().any(|n| n == "eq<point>"), "{f:?}");
    assert!(f.iter().any(|n| n == "point.x"), "{f:?}");
    let c = export("struct point  x: i32  y: f64\n: main ( -- ) 3 3 eq i32.to-str println ;");
    let f = functions(c.wasm.as_ref().unwrap());
    assert!(
        !f.iter().any(|n| n.starts_with("eq<") || n == "point.x"),
        "{f:?}"
    );
    let c = ok("struct node  v: i32  next: option node\n: same ( node node -- i32 ) eq ;\n: h ( str -- i32 ) hash ;");
    assert!(c.word("eq<node>").is_some());
    assert!(c.word("eq<option node>").is_some());
    assert!(c.word("hash<str>").unwrap().library);
}

#[test]
fn type_names_may_be_primitive_names() {
    // The prelude's `map K V` is one; `fold` is another primitive name.
    let c = ok("struct fold K V  k: K  v: V\n: f ( -- str ) 3 \"x\" fold.new fold.v ;\n: g ( fold i32 str -- i32 ) fold.k ;\n: h ( array i32 -- i32 ) 0 [ i32.add ] fold ;\n: m ( array i32 -- array i32 ) [ 1 i32.add ] map ;\n: n ( -- map i32 str ) map.make ( map i32 str ) ;");
    assert!(c.word("fold.new").is_some());
    assert_eq!(err("struct i32  x: i32"), "E_SYNTAX");
    assert_eq!(err("struct array  x: i32"), "E_SYNTAX");
    assert_eq!(err("union u | dup"), "E_SYNTAX");
}

#[test]
fn prelude_vec() {
    let c = ok(": main ( -- ) ;");
    let push = c.word("vec.push").expect("made at the end of the program");
    assert!(push.library && push.generic);
    let c = export(
        ": main ( -- ) vec.make ( vec i32 ) :> v  v 3 vec.push  v 0 vec.at i32.to-str println ;",
    );
    assert!(c.ok(), "{:?}", c.diagnostics);
    let f = functions(c.wasm.as_ref().unwrap());
    assert!(f.iter().any(|n| n == "vec.push<i32>"), "{f:?}");
    assert!(f.iter().any(|n| n == "vec.at<i32>"), "{f:?}");
    let c = export(": main ( -- ) 3 i32.to-str println ;");
    let f = functions(c.wasm.as_ref().unwrap());
    assert!(
        !f.iter()
            .any(|n| n.starts_with("vec.") || n.starts_with("chunk.")),
        "{f:?}"
    );
}

#[test]
fn prelude_map() {
    let c = ok(": main ( -- ) ;");
    let set = c.word("map.set").expect("made at the end of the program");
    assert!(set.library && set.generic);
    let c = export(": main ( -- ) map.make ( map str i32 ) :> m  m \"a\" 1 map.set  m map.len i32.to-str println ;");
    assert!(c.ok(), "{:?}", c.diagnostics);
    let f = functions(c.wasm.as_ref().unwrap());
    assert!(f.iter().any(|n| n == "map.set<str,i32>"), "{f:?}");
    let c = export(": main ( -- ) 3 i32.to-str println ;");
    let f = functions(c.wasm.as_ref().unwrap());
    assert!(!f.iter().any(|n| n.starts_with("map.")), "{f:?}");
}

#[test]
fn raw_words_need_the_marker() {
    assert_eq!(err(": f ( -- i32 ) 8 mem.alloc ;"), "E_RAW");
    assert_eq!(err(": f ( i32 -- i32 ) i32.load ;"), "E_RAW");
    assert_eq!(err(": f ( str -- i32 ) str.addr ;"), "E_RAW");
    assert_eq!(err(": f ( -- [ -- i32 ] ) [ 8 mem.alloc ] ;"), "E_RAW");
    assert_eq!(
        err(": f ( -- i32 ) 1 ;\ntest f : 0 i32.load drop f -> 1"),
        "E_RAW"
    );
    // `memory.size` takes and gives no address.
    ok(": f ( -- i32 ) memory.size ;");
    let c = ok("raw : peek ( i32 -- i32 ) i32.load8_u ;\n\
        export raw : buf ( -- i32 ) 8 mem.alloc ;\n\
        raw export : q ( -- [ -- i32 ] ) [ 8 mem.alloc ] ;\n\
        : f ( -- i32 ) buf peek ;\ntest f : f -> 0");
    assert!(c.word("peek").unwrap().raw && c.word("buf").unwrap().raw);
    assert!(!c.word("f").unwrap().raw);
    // The prelude's own words and generic instances are trusted.
    ok(": f ( -- i32 ) \"a\" \"b\" str.concat str.len ;\n: g ( -- i32 ) vec.make ( vec i32 ) :> v  v 1 vec.push  v vec.len ;");
}
