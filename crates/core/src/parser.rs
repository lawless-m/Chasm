//! Parser: tokens to top-level items.

use std::collections::HashMap;

use crate::ast::{Arm, Body, Item, Lit, Node, NodeKind, Variant};
use crate::diag::{codes, Diagnostic, Location};
use crate::lexer::{TokKind, Token};
use crate::types::{Effect, Ty};

pub fn parse(file: &str, toks: &[Token]) -> Result<Vec<Item>, Diagnostic> {
    parse_with(file, toks, &HashMap::new())
}

/// Parse with the arities of generic types declared before this text
/// (`pair` takes 2), so `pair i32 str` reads its arguments.
pub fn parse_with(
    file: &str,
    toks: &[Token],
    known: &HashMap<String, usize>,
) -> Result<Vec<Item>, Diagnostic> {
    let mut p = Parser {
        file,
        toks,
        pos: 0,
        known: known.clone(),
    };
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
    parse_repl_with(file, toks, &HashMap::new())
}

pub fn parse_repl_with(
    file: &str,
    toks: &[Token],
    known: &HashMap<String, usize>,
) -> Result<ReplInput, Diagnostic> {
    match toks.first() {
        None => Ok(ReplInput::Body(Vec::new())),
        Some(t)
            if [":", "export", "declare", "test", "struct", "union"]
                .iter()
                .any(|k| t.is(k)) =>
        {
            Ok(ReplInput::Items(parse_with(file, toks, known)?))
        }
        Some(_) => {
            let mut p = Parser {
                file,
                toks,
                pos: 0,
                known: known.clone(),
            };
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
    /// Arities of the generic types declared so far.
    known: HashMap<String, usize>,
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
        if t.is("union") {
            return self.union_item();
        }
        Err(self.err(
            format!(
                "expected `:`, `export`, `declare`, `test`, `struct` or `union` at top level, found `{}`",
                t.text
            ),
            self.loc(t),
        ))
    }

    /// After `struct`: the name, then `field: type` pairs while the next token
    /// is a field label. No terminator.
    fn struct_item(&mut self) -> Result<Item, Diagnostic> {
        let name = self.type_name("a struct name after `struct`", "a struct name")?;
        let n = name.text.as_str();
        let params = self.params(n)?;
        let fields = self.fields(n)?;
        Ok(Item::Struct {
            name: n.to_string(),
            params,
            fields,
            loc: self.loc(name),
        })
    }

    /// After `union`: the name, then one or more `| variant  field: type ...`
    /// groups. No terminator.
    fn union_item(&mut self) -> Result<Item, Diagnostic> {
        let name = self.type_name("a union name after `union`", "a union name")?;
        let n = name.text.as_str();
        let params = self.params(n)?;
        let mut variants: Vec<Variant> = Vec::new();
        while self.peek().is_some_and(|t| t.is("|")) {
            self.pos += 1;
            let v = self.type_name("a variant name after `|`", "a variant name")?;
            let vn = v.text.as_str();
            if vn == "else" || vn == "tag" || vn.contains('.') {
                return Err(self.err(
                    format!("`{vn}` cannot be a variant name: `else` is the catch-all arm of `match` and `{n}.tag` gives the variant"),
                    self.loc(v),
                ));
            }
            if variants.iter().any(|w| w.name == vn) {
                return Err(self.err(
                    format!("variant `{vn}` appears twice in `{n}`"),
                    self.loc(v),
                ));
            }
            let fields = self.fields(&format!("{n}.{vn}"))?;
            variants.push(Variant {
                name: vn.to_string(),
                fields,
                loc: self.loc(v),
            });
        }
        if variants.is_empty() {
            return Err(self.err(
                format!("a union needs at least one variant: `union {n} | a | b  field: i32`"),
                self.loc(name),
            ));
        }
        Ok(Item::Union {
            name: n.to_string(),
            params,
            variants,
            loc: self.loc(name),
        })
    }

    /// Type parameters after a struct or union name: uppercase-initial
    /// names. The type's arity is known from here on, its own fields
    /// included.
    fn params(&mut self, owner: &str) -> Result<Vec<String>, Diagnostic> {
        let mut params: Vec<String> = Vec::new();
        while let Some(t) = self.peek().filter(|t| {
            t.kind == TokKind::Word
                && t.text.starts_with(|c: char| c.is_ascii_uppercase())
                && !t.text.ends_with(':')
        }) {
            self.pos += 1;
            if params.contains(&t.text) {
                return Err(self.err(
                    format!("type parameter `{}` appears twice in `{owner}`", t.text),
                    self.loc(t),
                ));
            }
            params.push(t.text.clone());
        }
        self.known.insert(owner.to_string(), params.len());
        Ok(params)
    }

    /// A struct, union or variant name: lowercase, not a type keyword,
    /// number, primitive or punctuation.
    fn type_name(&mut self, what: &str, kind: &str) -> Result<&'a Token, Diagnostic> {
        let name = self.name(what)?;
        let n = name.text.as_str();
        if TYPE_KEYWORDS.contains(&n)
            || n.starts_with('\'')
            || parse_number(n).is_some()
            || crate::prims::is_builtin(n)
            || is_punct(n)
            || n == "|"
        {
            return Err(self.err(format!("`{n}` cannot be {kind}"), self.loc(name)));
        }
        if n.starts_with(|c: char| c.is_ascii_uppercase()) {
            let lower = n.to_ascii_lowercase();
            return Err(self.err(
                format!("`{n}` cannot be {kind}: a name starting with an uppercase letter is a type variable; write `{lower}`"),
                self.loc(name),
            ));
        }
        Ok(name)
    }

    /// `field: type` pairs while the next token is a field label.
    fn fields(&mut self, owner: &str) -> Result<Vec<(String, Ty, Location)>, Diagnostic> {
        let mut fields: Vec<(String, Ty, Location)> = Vec::new();
        while let Some(label) = self.peek().filter(|t| field_label(t).is_some()) {
            self.pos += 1;
            let f = field_label(label).unwrap();
            let loc = self.loc(label);
            if f == "new" || f.ends_with('!') {
                return Err(self.err(
                    format!(
                        "`{f}` cannot be a field name: `.new` makes a value and `!` marks a write"
                    ),
                    loc,
                ));
            }
            if fields.iter().any(|(g, _, _)| g == f) {
                return Err(self.err(format!("field `{f}` appears twice in `{owner}`"), loc));
            }
            let ty = self.ty()?;
            fields.push((f.to_string(), ty, loc));
        }
        Ok(fields)
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

    /// A type. Built-in types and struct names are lowercase, so a name
    /// starting with an uppercase ASCII letter is always a type variable
    /// (`T`, `Elem`), in every type position; it is never read as a struct.
    fn ty(&mut self) -> Result<Ty, Diagnostic> {
        let t = self.next("a type")?;
        let unknown = |p: &Self| {
            Diagnostic::error(
                codes::E_UNKNOWN_TYPE,
                format!(
                    "unknown type `{}` (types are i32 i64 f32 f64 str, `array T`, `[ effect ]`, a declared struct name, or a type variable such as `T`)",
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
            s if s.starts_with(|c: char| c.is_ascii_uppercase()) && !s.ends_with(':') => {
                Ty::Param(s.to_string())
            }
            s if !is_punct(s) && !s.ends_with(':') => {
                let arity = self.known.get(s).copied().unwrap_or(0);
                let mut args = Vec::new();
                for _ in 0..arity {
                    if self
                        .peek()
                        .is_none_or(|n| [")", "]", "--", ";"].iter().any(|k| n.is(k)))
                    {
                        return Err(Diagnostic::error(
                            codes::E_UNKNOWN_TYPE,
                            format!(
                                "`{s}` takes {arity} type argument{}",
                                if arity == 1 { "" } else { "s" }
                            ),
                            self.loc(t),
                        ));
                    }
                    args.push(self.ty()?);
                }
                Ty::Struct(s.to_string(), args)
            }
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
                    } else if text == "match" {
                        let mut arms = Vec::new();
                        while out.len() >= 2 {
                            let n = out.len();
                            let label = match (&out[n - 2].kind, &out[n - 1].kind) {
                                (NodeKind::Name(l), NodeKind::Quote(_)) => {
                                    match l.strip_suffix(':').filter(|l| !l.is_empty()) {
                                        Some(l) => l.to_string(),
                                        None => break,
                                    }
                                }
                                _ => break,
                            };
                            let Some(Node {
                                kind: NodeKind::Quote(body),
                                ..
                            }) = out.pop()
                            else {
                                unreachable!()
                            };
                            let at = out.pop().unwrap().loc;
                            arms.push(Arm {
                                label,
                                body,
                                loc: at,
                            });
                        }
                        if arms.is_empty() {
                            return Err(self.err(
                                "`match` takes labelled arms written directly before it, e.g. `s circle: [ ... ] rect: [ ... ] match`",
                                loc,
                            ));
                        }
                        arms.reverse();
                        NodeKind::Match(arms)
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
    fn type_parameters_and_application() {
        let Item::Struct { params, .. } = &p("struct pair T U  first: T  second: U")[0] else {
            panic!()
        };
        assert_eq!(params, &["T", "U"]);
        let items = p(
            "struct pair T U  first: T  second: U\n: f ( pair i32 str -- array pair i32 str ) ;",
        );
        let Item::Def {
            effect: Some(e), ..
        } = &items[1]
        else {
            panic!()
        };
        assert_eq!(
            e.inputs,
            [Ty::Struct("pair".into(), vec![Ty::I32, Ty::Str])]
        );
        assert_eq!(e.to_string(), "( pair i32 str -- array pair i32 str )");
        let items = p("union list T | nil | cons  head: T  tail: list T\nstruct pair T U  a: T  b: U\n: g ( pair list i32 str -- ) drop ;");
        let Item::Def {
            effect: Some(e), ..
        } = &items[2]
        else {
            panic!()
        };
        assert_eq!(
            e.inputs,
            [Ty::Struct(
                "pair".into(),
                vec![Ty::Struct("list".into(), vec![Ty::I32]), Ty::Str]
            )]
        );
        assert_eq!(
            perr("struct pair T U  a: T  b: U\n: h ( pair i32 ) ;").code,
            codes::E_UNKNOWN_TYPE
        );
        assert_eq!(perr("struct p T T  x: T").code, codes::E_SYNTAX);
    }

    #[test]
    fn match_arms() {
        let items =
            p(": f ( shape -- f64 ) circle: [ 1.0 ] rect: [ f64.mul ] else: [ drop 0.0 ] match ;");
        let Item::Def { body, .. } = &items[0] else {
            panic!()
        };
        assert_eq!(body.len(), 1);
        let NodeKind::Match(arms) = &body[0].kind else {
            panic!()
        };
        let labels: Vec<&str> = arms.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["circle", "rect", "else"]);
        assert_eq!(arms[2].body.len(), 2);
        for bad in [": f ( -- ) match ;", ": f ( -- ) [ 1 ] match ;"] {
            assert_eq!(perr(bad).code, codes::E_SYNTAX, "{bad}");
        }
        let Item::Def { body, .. } = &p(": f ( -- ) 1 a: [ 2 ] match ;")[0] else {
            panic!()
        };
        assert_eq!(body.len(), 2);
        assert!(matches!(body[0].kind, NodeKind::Lit(_)));
        let Ok(ReplInput::Body(b)) = parse_repl("t", &lex("t", "s a: [ 1 ] match").unwrap()) else {
            panic!()
        };
        assert!(matches!(b[1].kind, NodeKind::Match(_)));
    }

    #[test]
    fn unions() {
        let Item::Union { name, variants, .. } =
            &p("union shape | circle  r: f64 | rect  w: f64  h: f64 | empty")[0]
        else {
            panic!()
        };
        assert_eq!(name, "shape");
        let shape: Vec<(&str, usize)> = variants
            .iter()
            .map(|v| (v.name.as_str(), v.fields.len()))
            .collect();
        assert_eq!(shape, [("circle", 1), ("rect", 2), ("empty", 0)]);
        assert_eq!(variants[1].fields[1].1, Ty::F64);
        for bad in [
            "union e",
            "union e | else",
            "union e | a | a",
            "union e | a  x: i32  x: f64",
            "union e | tag",
        ] {
            assert_eq!(perr(bad).code, codes::E_SYNTAX, "{bad}");
        }
        assert_eq!(p("union s | a\n: f ( s -- s ) ;").len(), 2);
        assert!(matches!(
            parse_repl("t", &lex("t", "union s | a").unwrap()),
            Ok(ReplInput::Items(_))
        ));
        assert!(!crate::repl::needs_more("union s | a"));
    }

    #[test]
    fn type_params() {
        let Item::Def {
            effect: Some(e), ..
        } = &p(": f ( T -- T T ) dup ;")[0]
        else {
            panic!()
        };
        assert_eq!(e.inputs, [Ty::Param("T".into())]);
        let Item::Def {
            effect: Some(e), ..
        } = &p(": g ( array T [ T -- U ] -- U ) drop drop ;")[0]
        else {
            panic!()
        };
        assert_eq!(e.to_string(), "( array T [ T -- U ] -- U )");
        assert_eq!(e.params(), ["T", "U"]);
        let Item::Def { body, .. } = &p(": h ( -- ) ( a b ) ;")[0] else {
            panic!()
        };
        assert!(
            matches!(&body[0].kind, NodeKind::Assert(tys) if tys == &[Ty::Struct("a".into(), Vec::new()), Ty::Struct("b".into(), Vec::new())]),
            "{body:?}"
        );
        assert_eq!(perr("struct Point  x: i32").code, codes::E_SYNTAX);
        let Item::Struct { fields, .. } = &p("struct p  next: array T")[0] else {
            panic!()
        };
        assert_eq!(fields[0].1, Ty::Array(Box::new(Ty::Param("T".into()))));
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
        assert_eq!(e.inputs, vec![Ty::Struct("point".into(), Vec::new())]);

        let items = p("struct node  next: node  v: i32");
        let Item::Struct { fields, .. } = &items[0] else {
            panic!()
        };
        assert_eq!(fields[0].1, Ty::Struct("node".into(), Vec::new()));
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
            NodeKind::Assert(vec![
                Ty::Struct("a".into(), Vec::new()),
                Ty::Struct("b".into(), Vec::new())
            ])
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
