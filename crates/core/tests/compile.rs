use chasm_core::{compile, Options, Source};

fn ok(src: &str) -> chasm_core::Compilation {
    let c = compile(
        &[Source::new("t.chasm", src)],
        &Options {
            prelude: true,
            test_exports: true,
            export: false,
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
    use chasm_core::ast::{Node, NodeKind};
    use chasm_core::check::{compile_body, Ctx, Mode, Origin, Word, WordKind};
    use chasm_core::module::{assemble_step, export_name};
    use chasm_core::types::{Effect, Ty};
    use chasm_core::Location;

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
            origin: Origin::User,
            kind: WordKind::Named,
            loc: Location::default(),
            callees: vec![],
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
    chasm_core::validate(&bytes).unwrap();
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
    let chasm = |n: &str| ("chasm".to_string(), n.to_string());
    assert_eq!(
        imports,
        vec![chasm("ring_enter"), chasm("memory"), chasm("table")]
    );
    assert_eq!(exports, vec![export_name(0), export_name(1)]);
    assert_eq!(exports, vec!["w0", "w1"]);

    let shared = assemble_step(&mut ctx, &[0, 1], true);
    chasm_core::validate(&shared).unwrap();
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

fn export(src: &str) -> chasm_core::Compilation {
    compile(
        &[Source::new("t.chasm", src)],
        &Options {
            prelude: true,
            test_exports: false,
            export: true,
        },
    )
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
