//! Parser: tokens to top-level items.

use crate::ast::{Body, Item, Lit, Node, NodeKind};
use crate::diag::{codes, Diagnostic, Location};
use crate::lexer::{TokKind, Token};
use crate::types::{Effect, Ty};

pub fn parse(file: &str, toks: &[Token]) -> Result<Vec<Item>, Diagnostic> {
    let mut p = Parser { file, toks, pos: 0 };
    let mut items = Vec::new();
    while p.pos < toks.len() {
        items.push(p.item()?);
    }
    Ok(items)
}

/// A REPL chunk: top-level forms as in a file, or one bare body (a line).
#[derive(Debug, Clone, PartialEq)]
pub enum ReplInput {
    Items(Vec<Item>),
    Body(Body),
}

/// Parse a REPL chunk. A chunk whose first token is `:`, `export`,
/// `declare` or `test` is parsed exactly like a file; anything else is one
/// body running to the end of the input.
pub fn parse_repl(file: &str, toks: &[Token]) -> Result<ReplInput, Diagnostic> {
    match toks.first() {
        None => Ok(ReplInput::Body(Vec::new())),
        Some(t)
            if [":", "export", "declare", "test", "struct"]
                .iter()
                .any(|k| t.is(k)) =>
        {
            Ok(ReplInput::Items(parse(file, toks)?))
        }
        Some(_) => {
            let mut p = Parser { file, toks, pos: 0 };
            Ok(ReplInput::Body(p.body_inner(&[], true)?))
        }
    }
}

/// Combinators and how many quotations they take.
pub fn combinator_arity(name: &str) -> Option<usize> {
    Some(match name {
        "if" | "while" | "until" => 2,
        "when" | "unless" | "times" | "each" | "map" | "filter" | "fold" => 1,
        _ => return None,
    })
}

/// An integer token's value: `None` if it is not an integer, `Some(None)`
/// if it is too large to represent at all.
fn int_value(text: &str) -> Option<Option<i128>> {
    let (neg, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let mag = if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        u128::from_str_radix(hex, 16).ok()
    } else if !body.is_empty() && body.chars().all(|c| c.is_ascii_digit()) {
        body.parse::<u128>().ok()
    } else {
        return None;
    };
    Some(mag.map(|m| if neg { -(m as i128) } else { m as i128 }))
}

/// Parse a numeric literal: an `i32` integer or an `f64`. `None` if the
/// token is not a number at all.
pub fn parse_number(text: &str) -> Option<Result<Lit, String>> {
    if let Some(v) = int_value(text) {
        return Some(match v {
            Some(v) if v >= i32::MIN as i128 && v <= u32::MAX as i128 => Ok(Lit::I32(v as i32)),
            _ => Err(format!(
                "literal `{text}` does not fit in i32; write `{text} i64` for an i64"
            )),
        });
    }
    let body = text.strip_prefix('-').unwrap_or(text);
    if !body.starts_with(|c: char| c.is_ascii_digit())
        || !body.contains(['.', 'e', 'E'])
        || !body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
    {
        return None;
    }
    let v: f64 = body.parse().ok()?;
    let v = if text.starts_with('-') { -v } else { v };
    Some(if v.is_infinite() {
        Err(format!("literal `{text}` does not fit in f64"))
    } else {
        Ok(Lit::F64(v))
    })
}

/// Parse an integer token as the `i64` literal written `text i64`. `None`
/// if the token is not an integer.
pub fn parse_i64(text: &str) -> Option<Result<Lit, String>> {
    Some(match int_value(text)? {
        Some(v) if v >= i64::MIN as i128 && v <= u64::MAX as i128 => Ok(Lit::I64(v as i64)),
        _ => Err(format!("literal `{text} i64` does not fit in i64")),
    })
}

const TYPE_KEYWORDS: [&str; 6] = ["i32", "i64", "f32", "f64", "str", "array"];

fn is_punct(s: &str) -> bool {
    matches!(s, "(" | ")" | "[" | "]" | "--" | ";" | ":" | ":>" | "->")
}

/// `x:` as a field label gives `x`.
fn field_label(t: &Token) -> Option<&str> {
    if t.kind != TokKind::Word {
        return None;
    }
    t.text
        .strip_suffix(':')
        .filter(|n| !n.is_empty() && !n.starts_with(':'))
}

struct Parser<'a> {
    file: &'a str,
    toks: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn loc(&self, t: &Token) -> Location {
        t.location(self.file)
    }

    fn eof_loc(&self) -> Location {
        match self.toks.last() {
            Some(t) => Location {
                token: String::new(),
                ..self.loc(t)
            },
            None => Location {
                file: self.file.to_string(),
                line: 1,
                column: 1,
                token: String::new(),
            },
        }
    }

    fn err(&self, msg: impl Into<String>, loc: Location) -> Diagnostic {
        Diagnostic::error(codes::E_SYNTAX, msg, loc)
    }

    fn next(&mut self, what: &str) -> Result<&'a Token, Diagnostic> {
        match self.toks.get(self.pos) {
            Some(t) => {
                self.pos += 1;
                Ok(t)
            }
            None => Err(self.err(
                format!("unexpected end of file, expected {what}"),
                self.eof_loc(),
            )),
        }
    }

    fn peek(&self) -> Option<&'a Token> {
        self.toks.get(self.pos)
    }

    fn name(&mut self, what: &str) -> Result<&'a Token, Diagnostic> {
        let t = self.next(what)?;
        if t.kind != TokKind::Word {
            return Err(self.err(format!("expected {what}, found a string"), self.loc(t)));
        }
        Ok(t)
    }

    fn item(&mut self) -> Result<Item, Diagnostic> {
        let t = self.next("a definition")?;
        if t.is("export") {
            let colon = self.next("`:` after `export`")?;
            if !colon.is(":") {
                return Err(self.err("expected `:` after `export`", self.loc(colon)));
            }
            return self.def(true);
        }
        if t.is(":") {
            return self.def(false);
        }
        if t.is("declare") {
            let name = self.name("a word name after `declare`")?;
            let open = self.next("an effect after the declared name")?;
            if !open.is("(") {
                return Err(self.err(
                    format!(
                        "expected an effect `( ... -- ... )` after `declare {}`",
                        name.text
                    ),
                    self.loc(open),
                ));
            }
            let effect = self.effect(open)?;
            return Ok(Item::Declare {
                name: name.text.clone(),
                effect,
                loc: self.loc(name),
            });
        }
        if t.is("test") {
            let word = self.name("a word name after `test`")?;
            let colon = self.next("`:` after the tested word")?;
            if !colon.is(":") {
                return Err(self.err(
                    "expected `:` in `test word : body -> expected`",
                    self.loc(colon),
                ));
            }
            let body = self.body(&["->"])?;
            let mut expected = Vec::new();
            while let Some(t) = self.peek() {
                self.pos += 1;
                let lit = match &t.kind {
                    TokKind::Str(s) => Lit::Str(s.clone()),
                    TokKind::Word => match self.number(t) {
                        Some(l) => l?,
                        None => {
                            self.pos -= 1;
                            break;
                        }
                    },
                };
                expected.push((lit, self.loc(t)));
            }
            return Ok(Item::Test {
                word: word.text.clone(),
                body,
                expected,
                loc: self.loc(t),
            });
        }
        if t.is("struct") {
            return self.struct_item();
        }
        Err(self.err(
            format!(
                "expected `:`, `export`, `declare`, `test` or `struct` at top level, found `{}`",
                t.text
            ),
            self.loc(t),
        ))
    }

    /// After `struct`: the name, then `field: type` pairs while the next token
    /// is a field label. No terminator.
    fn struct_item(&mut self) -> Result<Item, Diagnostic> {
        let name = self.name("a struct name after `struct`")?;
        let n = name.text.as_str();
        if TYPE_KEYWORDS.contains(&n)
            || n.starts_with('\'')
            || parse_number(n).is_some()
            || crate::prims::is_builtin(n)
            || is_punct(n)
        {
            return Err(self.err(format!("`{n}` cannot be a struct name"), self.loc(name)));
        }
        let mut fields: Vec<(String, Ty, Location)> = Vec::new();
        while let Some(label) = self.peek().filter(|t| field_label(t).is_some()) {
            self.pos += 1;
            let f = field_label(label).unwrap();
            let loc = self.loc(label);
            if f == "new" || f.ends_with('!') {
                return Err(self.err(
                    format!("`{f}` cannot be a field name: `{n}.new` is the constructor and `!` marks a write"),
                    loc,
                ));
            }
            if fields.iter().any(|(g, _, _)| g == f) {
                return Err(self.err(format!("field `{f}` appears twice in `{n}`"), loc));
            }
            let ty = self.ty()?;
            fields.push((f.to_string(), ty, loc));
        }
        Ok(Item::Struct {
            name: n.to_string(),
            fields,
            loc: self.loc(name),
        })
    }

    /// The numeric literal at `t` (already consumed), or `None`. An integer
    /// followed by `i64` is one `i64` literal, and the `i64` is consumed.
    fn number(&mut self, t: &Token) -> Option<Result<Lit, Diagnostic>> {
        let loc = self.loc(t);
        let err = |m| Diagnostic::error(codes::E_LITERAL_RANGE, m, loc.clone());
        if self.peek().is_some_and(|n| n.is("i64")) {
            if let Some(r) = parse_i64(&t.text) {
                self.pos += 1;
                return Some(r.map_err(err));
            }
        }
        parse_number(&t.text).map(|r| r.map_err(err))
    }

    fn def(&mut self, export: bool) -> Result<Item, Diagnostic> {
        let name = self.name("a word name after `:`")?;
        let effect = match self.peek() {
            Some(t) if t.is("(") => {
                self.pos += 1;
                Some(self.effect(t)?)
            }
            _ => None,
        };
        let body = self.body(&[";"])?;
        Ok(Item::Def {
            name: name.text.clone(),
            effect,
            body,
            export,
            loc: self.loc(name),
        })
    }

    /// Parse after `(` up to `)`, requiring `--`.
    fn effect(&mut self, open: &Token) -> Result<Effect, Diagnostic> {
        let inputs = self.types(&["--", ")"])?;
        let sep = self.next("`--` or `)`")?;
        if !sep.is("--") {
            return Err(self.err(
                "an effect needs `--` between inputs and outputs, e.g. `( i32 -- i32 )`",
                self.loc(open),
            ));
        }
        let outputs = self.types(&[")"])?;
        self.next("`)`")?;
        Ok(Effect::new(inputs, outputs))
    }

    /// Parse types until (not consuming) one of `stops`.
    fn types(&mut self, stops: &[&str]) -> Result<Vec<Ty>, Diagnostic> {
        let mut out = Vec::new();
        loop {
            let Some(t) = self.peek() else {
                return Err(self.err(
                    format!(
                        "unexpected end of file, expected one of {}",
                        stops.join(" ")
                    ),
                    self.eof_loc(),
                ));
            };
            if stops.iter().any(|s| t.is(s)) {
                return Ok(out);
            }
            out.push(self.ty()?);
        }
    }

    fn ty(&mut self) -> Result<Ty, Diagnostic> {
        let t = self.next("a type")?;
        let unknown = |p: &Self| {
            Diagnostic::error(
                codes::E_UNKNOWN_TYPE,
                format!(
                    "unknown type `{}` (types are i32 i64 f32 f64 str, `array T`, `[ effect ]` or a declared struct name)",
                    t.text
                ),
                p.loc(t),
            )
        };
        if t.kind != TokKind::Word {
            return Err(unknown(self));
        }
        Ok(match t.text.as_str() {
            "i32" => Ty::I32,
            "i64" => Ty::I64,
            "f32" => Ty::F32,
            "f64" => Ty::F64,
            "str" => Ty::Str,
            "array" => {
                let elem = self.ty()?;
                if matches!(elem, Ty::Array(_)) {
                    return Err(Diagnostic::error(
                        codes::E_UNKNOWN_TYPE,
                        "nested arrays are not supported in v1",
                        self.loc(t),
                    ));
                }
                Ty::Array(Box::new(elem))
            }
            "[" => {
                let inputs = self.types(&["--", "]"])?;
                let sep = self.next("`--`")?;
                if !sep.is("--") {
                    return Err(self.err(
                        "a quotation type needs `--`, e.g. `[ i32 -- i32 ]`",
                        self.loc(t),
                    ));
                }
                let outputs = self.types(&["]"])?;
                self.next("`]`")?;
                Ty::Quot(Box::new(Effect::new(inputs, outputs)))
            }
            s if !is_punct(s) && !s.ends_with(':') => Ty::Struct(s.to_string()),
            _ => return Err(unknown(self)),
        })
    }

    /// Parse body nodes until (and consuming) one of `ends`.
    fn body(&mut self, ends: &[&str]) -> Result<Body, Diagnostic> {
        self.body_inner(ends, false)
    }

    /// As `body`; with `to_end`, end of input also ends the body.
    fn body_inner(&mut self, ends: &[&str], to_end: bool) -> Result<Body, Diagnostic> {
        let mut out: Body = Vec::new();
        loop {
            if to_end && self.pos >= self.toks.len() {
                return Ok(out);
            }
            let t = self.next(&format!("`{}`", ends.join("` or `")))?;
            let loc = self.loc(t);
            if t.kind == TokKind::Word && ends.contains(&t.text.as_str()) {
                return Ok(out);
            }
            let kind = match &t.kind {
                TokKind::Str(s) => NodeKind::Lit(Lit::Str(s.clone())),
                TokKind::Word => {
                    let text = t.text.as_str();
                    if let Some(l) = self.number(t) {
                        NodeKind::Lit(l?)
                    } else if text == "[" {
                        NodeKind::Quote(self.body(&["]"])?)
                    } else if text == "(" {
                        let tys = self.types(&[")", "--"])?;
                        let close = self.next("`)`")?;
                        if close.is("--") {
                            return Err(self.err(
                                "an effect `( .. -- .. )` is not allowed inside a body; a stack assertion lists types without `--`",
                                loc,
                            ));
                        }
                        NodeKind::Assert(tys)
                    } else if text == ":>" {
                        let n = self.name("a local name after `:>`")?;
                        let (name, mutable) = match n.text.strip_suffix('!') {
                            Some(base) => (base.to_string(), true),
                            None => (n.text.clone(), false),
                        };
                        if name.is_empty() || parse_number(&name).is_some() {
                            return Err(self.err(
                                format!("`{}` is not a valid local name", n.text),
                                self.loc(n),
                            ));
                        }
                        NodeKind::Bind { name, mutable }
                    } else if text == "leave" {
                        NodeKind::Leave
                    } else if let Some(arity) = combinator_arity(text) {
                        let mut quotes = Vec::new();
                        for _ in 0..arity {
                            match out.pop() {
                                Some(Node {
                                    kind: NodeKind::Quote(b),
                                    ..
                                }) => quotes.push(b),
                                _ => {
                                    return Err(self.err(
                                        format!(
                                            "`{text}` takes {arity} quotation{} written directly before it, e.g. {}",
                                            if arity == 1 { "" } else { "s" },
                                            combinator_example(text)
                                        ),
                                        loc,
                                    ))
                                }
                            }
                        }
                        quotes.reverse();
                        let mut q = quotes.into_iter();
                        let mut take = || q.next().unwrap();
                        match text {
                            "if" => NodeKind::If(take(), take()),
                            "when" => NodeKind::When(take()),
                            "unless" => NodeKind::Unless(take()),
                            "while" => NodeKind::While(take(), take()),
                            "until" => NodeKind::Until(take(), take()),
                            "times" => NodeKind::Times(take()),
                            "each" => NodeKind::Each(take()),
                            "map" => NodeKind::Map(take()),
                            "filter" => NodeKind::Filter(take()),
                            "fold" => NodeKind::Fold(take()),
                            _ => unreachable!(),
                        }
                    } else if let Some(name) = text.strip_prefix('\'') {
                        if name.is_empty() {
                            return Err(self.err("expected a word name after `'`", loc));
                        }
                        NodeKind::Tick(name.to_string())
                    } else if matches!(text, "]" | ")" | ";" | ":" | "--" | "->") {
                        return Err(self.err(format!("unexpected `{text}`"), loc));
                    } else {
                        NodeKind::Name(text.to_string())
                    }
                }
            };
            out.push(Node { kind, loc });
        }
    }
}

fn combinator_example(name: &str) -> &'static str {
    match name {
        "if" => "`cond [ then ] [ else ] if`",
        "when" => "`cond [ body ] when`",
        "unless" => "`cond [ body ] unless`",
        "while" => "`[ cond ] [ body ] while`",
        "until" => "`[ body ] [ cond ] until`",
        "times" => "`n [ body ] times`",
        "each" => "`arr [ body ] each`",
        "map" => "`arr [ body ] map`",
        "filter" => "`arr [ pred ] filter`",
        "fold" => "`arr init [ body ] fold`",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    fn p(src: &str) -> Vec<Item> {
        parse("t", &lex("t", src).unwrap()).unwrap()
    }

    fn perr(src: &str) -> Diagnostic {
        parse("t", &lex("t", src).unwrap()).unwrap_err()
    }

    #[test]
    fn structs() {
        let items = p("struct point  x: i32  y: f64");
        let Item::Struct { name, fields, .. } = &items[0] else {
            panic!("{items:?}")
        };
        assert_eq!(name, "point");
        let tys: Vec<_> = fields
            .iter()
            .map(|(n, t, _)| (n.as_str(), t.clone()))
            .collect();
        assert_eq!(tys, [("x", Ty::I32), ("y", Ty::F64)]);

        let items = p("struct seg  a: point  b: point\n: f ( point -- i32 ) point.x ;");
        assert_eq!(items.len(), 2);
        let Item::Def {
            effect: Some(e), ..
        } = &items[1]
        else {
            panic!()
        };
        assert_eq!(e.inputs, vec![Ty::Struct("point".into())]);

        let items = p("struct node  next: node  v: i32");
        let Item::Struct { fields, .. } = &items[0] else {
            panic!()
        };
        assert_eq!(fields[0].1, Ty::Struct("node".into()));
        assert!(matches!(&p("struct empty")[0], Item::Struct { fields, .. } if fields.is_empty()));

        for bad in [
            "struct t  new: i32",
            "struct t  x: i32  x: f64",
            "struct i32  x: i32",
            "struct t  x!: i32",
            "struct",
        ] {
            assert_eq!(perr(bad).code, codes::E_SYNTAX, "{bad}");
        }
        assert!(matches!(
            parse_repl("t", &lex("t", "struct p  x: i32").unwrap()).unwrap(),
            ReplInput::Items(_)
        ));
        let items = p(": f ( i32 -- i32 ) ( a b ) ;");
        let Item::Def { body, .. } = &items[0] else {
            panic!()
        };
        assert_eq!(
            body[0].kind,
            NodeKind::Assert(vec![Ty::Struct("a".into()), Ty::Struct("b".into())])
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(parse_number("42"), Some(Ok(Lit::I32(42))));
        assert_eq!(parse_number("-7"), Some(Ok(Lit::I32(-7))));
        assert_eq!(parse_number("0xFF"), Some(Ok(Lit::I32(255))));
        assert_eq!(parse_number("0xFFFFFFFF"), Some(Ok(Lit::I32(-1))));
        assert_eq!(parse_number("42i64"), None);
        assert_eq!(parse_i64("42"), Some(Ok(Lit::I64(42))));
        assert_eq!(parse_i64("4294967295"), Some(Ok(Lit::I64(4294967295))));
        assert_eq!(parse_i64("1.5"), None);
        assert_eq!(parse_number("1.5"), Some(Ok(Lit::F64(1.5))));
        assert_eq!(parse_number("2e10"), Some(Ok(Lit::F64(2e10))));
        assert_eq!(parse_number("1.5f32"), None);
        assert!(matches!(parse_number("4294967296"), Some(Err(_))));
        assert_eq!(parse_number("2dup"), None);
        assert_eq!(parse_number("-"), None);
        assert_eq!(parse_number("-rot"), None);
    }

    #[test]
    fn def_and_test() {
        let items = p(
            ": sq ( i32 -- i32 ) dup i32.mul ;\ntest sq : 3 sq -> 9\ndeclare f ( str -- i32 i32 )",
        );
        assert_eq!(items.len(), 3);
        match &items[1] {
            Item::Test { expected, .. } => assert_eq!(expected[0].0, Lit::I32(9)),
            _ => panic!(),
        }
    }

    fn repl(src: &str) -> Result<ReplInput, Diagnostic> {
        parse_repl("t", &lex("t", src).unwrap())
    }

    #[test]
    fn repl_chunks() {
        assert!(matches!(repl("3 sq").unwrap(), ReplInput::Body(b) if b.len() == 2));
        assert!(matches!(repl(": f ( -- ) ;").unwrap(), ReplInput::Items(i) if i.len() == 1));
        match repl("1 [ 1 ] [ 2 ] if").unwrap() {
            ReplInput::Body(b) => assert!(matches!(b.last().unwrap().kind, NodeKind::If(..))),
            other => panic!("{other:?}"),
        }
        assert_eq!(repl("[ 1").unwrap_err().code, "E_SYNTAX");
        assert_eq!(repl("").unwrap(), ReplInput::Body(vec![]));
        assert_eq!(repl(": f ( -- ) ; 3").unwrap_err().code, "E_SYNTAX");
    }

    #[test]
    fn combinators_take_quotes() {
        let items = p(": f ( i32 -- i32 ) 0 i32.gt_s [ 1 ] [ 2 ] if ;");
        let Item::Def { body, .. } = &items[0] else {
            panic!()
        };
        assert!(matches!(body.last().unwrap().kind, NodeKind::If(..)));
        let e = parse("t", &lex("t", ": f ( -- ) 1 if ;").unwrap()).unwrap_err();
        assert_eq!(e.code, "E_SYNTAX");
    }
}
