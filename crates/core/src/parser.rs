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

/// Combinators and how many quotations they take.
pub fn combinator_arity(name: &str) -> Option<usize> {
    Some(match name {
        "if" | "while" | "until" => 2,
        "when" | "unless" | "times" | "each" | "map" | "filter" | "fold" => 1,
        _ => return None,
    })
}

/// Parse a numeric literal. `None` if the token is not a number at all.
pub fn parse_number(text: &str) -> Option<Result<Lit, String>> {
    let (neg, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    if !body.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let range_err = |ty: &str| Err(format!("literal `{text}` does not fit in {ty}"));
    // Integers, optionally hex, optionally `i64`.
    let (digits, is_i64) = match body.strip_suffix("i64") {
        Some(d) => (d, true),
        None => (body, false),
    };
    let int = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            None
        } else {
            Some(u128::from_str_radix(hex, 16).ok())
        }
    } else if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        Some(digits.parse::<u128>().ok())
    } else {
        None
    };
    if let Some(mag) = int {
        let Some(mag) = mag else {
            return Some(range_err(if is_i64 { "i64" } else { "i32" }));
        };
        let mag = mag as i128;
        let v = if neg { -mag } else { mag };
        return Some(if is_i64 {
            if v < i64::MIN as i128 || v > u64::MAX as i128 {
                range_err("i64")
            } else {
                Ok(Lit::I64(v as i64))
            }
        } else if v < i32::MIN as i128 || v > u32::MAX as i128 {
            range_err("i32")
        } else {
            Ok(Lit::I32(v as i32))
        });
    }
    // Floats: decimal with a point or exponent, optionally `f32`/`f64`.
    let (fdigits, is_f32) = if let Some(d) = body.strip_suffix("f32") {
        (d, true)
    } else if let Some(d) = body.strip_suffix("f64") {
        (d, false)
    } else {
        (body, false)
    };
    let looks_float = fdigits
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'));
    if !looks_float {
        return None;
    }
    let explicit = is_f32 || body.ends_with("f64");
    if !explicit && !fdigits.contains(['.', 'e', 'E']) {
        return None;
    }
    let v: f64 = match fdigits.parse() {
        Ok(v) => v,
        Err(_) => return None,
    };
    let v = if neg { -v } else { v };
    Some(if is_f32 {
        let f = v as f32;
        if f.is_infinite() && v.is_finite() {
            range_err("f32")
        } else {
            Ok(Lit::F32(f))
        }
    } else if v.is_infinite() {
        range_err("f64")
    } else {
        Ok(Lit::F64(v))
    })
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
                let lit = match &t.kind {
                    TokKind::Str(s) => Lit::Str(s.clone()),
                    TokKind::Word => match parse_number(&t.text) {
                        Some(Ok(l)) => l,
                        Some(Err(m)) => {
                            return Err(Diagnostic::error(codes::E_LITERAL_RANGE, m, self.loc(t)))
                        }
                        None => break,
                    },
                };
                expected.push((lit, self.loc(t)));
                self.pos += 1;
            }
            return Ok(Item::Test {
                word: word.text.clone(),
                body,
                expected,
                loc: self.loc(t),
            });
        }
        Err(self.err(
            format!(
                "expected `:`, `export`, `declare` or `test` at top level, found `{}`",
                t.text
            ),
            self.loc(t),
        ))
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
                    "unknown type `{}` (types are i32 i64 f32 f64 str, `array T` and `[ effect ]`)",
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
            _ => return Err(unknown(self)),
        })
    }

    /// Parse body nodes until (and consuming) one of `ends`.
    fn body(&mut self, ends: &[&str]) -> Result<Body, Diagnostic> {
        let mut out: Body = Vec::new();
        loop {
            let t = self.next(&format!("`{}`", ends.join("` or `")))?;
            let loc = self.loc(t);
            if t.kind == TokKind::Word && ends.contains(&t.text.as_str()) {
                return Ok(out);
            }
            let kind = match &t.kind {
                TokKind::Str(s) => NodeKind::Lit(Lit::Str(s.clone())),
                TokKind::Word => {
                    let text = t.text.as_str();
                    if let Some(n) = parse_number(text) {
                        match n {
                            Ok(l) => NodeKind::Lit(l),
                            Err(m) => {
                                return Err(Diagnostic::error(codes::E_LITERAL_RANGE, m, loc))
                            }
                        }
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

    #[test]
    fn numbers() {
        assert_eq!(parse_number("42"), Some(Ok(Lit::I32(42))));
        assert_eq!(parse_number("-7"), Some(Ok(Lit::I32(-7))));
        assert_eq!(parse_number("0xFF"), Some(Ok(Lit::I32(255))));
        assert_eq!(parse_number("0xFFFFFFFF"), Some(Ok(Lit::I32(-1))));
        assert_eq!(parse_number("42i64"), Some(Ok(Lit::I64(42))));
        assert_eq!(parse_number("1.5"), Some(Ok(Lit::F64(1.5))));
        assert_eq!(parse_number("2e10"), Some(Ok(Lit::F64(2e10))));
        assert_eq!(parse_number("1.5f32"), Some(Ok(Lit::F32(1.5))));
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
