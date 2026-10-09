//! Lexer: whitespace-separated tokens, `#` comments, double-quoted strings.

use crate::diag::{codes, Diagnostic, Location};

#[derive(Debug, Clone, PartialEq)]
pub enum TokKind {
    /// Any whitespace-delimited token that is not a string literal.
    Word,
    /// A string literal; the payload is the unescaped content.
    Str(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokKind,
    /// Source text of the token, as written.
    pub text: String,
    pub line: u32,
    pub column: u32,
}

impl Token {
    pub fn location(&self, file: &str) -> Location {
        Location {
            file: file.to_string(),
            line: self.line,
            column: self.column,
            token: self.text.clone(),
        }
    }

    pub fn is(&self, word: &str) -> bool {
        self.kind == TokKind::Word && self.text == word
    }
}

pub fn lex(file: &str, src: &str) -> Result<Vec<Token>, Diagnostic> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    let mut line = 1u32;
    let mut col = 1u32;

    let advance = |i: &mut usize, line: &mut u32, col: &mut u32, c: char| {
        *i += 1;
        if c == '\n' {
            *line += 1;
            *col = 1;
        } else {
            *col += 1;
        }
    };

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            advance(&mut i, &mut line, &mut col, c);
            continue;
        }
        let (tline, tcol) = (line, col);
        if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                {
                    let ch = chars[i];
                    advance(&mut i, &mut line, &mut col, ch);
                }
            }
            continue;
        }
        if c == '"' {
            let start = i;
            advance(&mut i, &mut line, &mut col, c);
            let mut content = String::new();
            let mut closed = false;
            while i < chars.len() {
                let c = chars[i];
                if c == '"' {
                    advance(&mut i, &mut line, &mut col, c);
                    closed = true;
                    break;
                }
                if c == '\\' {
                    let loc = Location {
                        file: file.to_string(),
                        line,
                        column: col,
                        token: "\\".into(),
                    };
                    advance(&mut i, &mut line, &mut col, c);
                    let Some(&e) = chars.get(i) else { break };
                    advance(&mut i, &mut line, &mut col, e);
                    match e {
                        '"' => content.push('"'),
                        '\\' => content.push('\\'),
                        'n' => content.push('\n'),
                        't' => content.push('\t'),
                        'r' => content.push('\r'),
                        'u' => {
                            if chars.get(i) != Some(&'{') {
                                return Err(Diagnostic::error(
                                    codes::E_LEX,
                                    "expected `{` after `\\u` in string escape",
                                    loc,
                                ));
                            }
                            advance(&mut i, &mut line, &mut col, '{');
                            let mut hex = String::new();
                            while i < chars.len() && chars[i] != '}' {
                                hex.push(chars[i]);
                                {
                                    let ch = chars[i];
                                    advance(&mut i, &mut line, &mut col, ch);
                                }
                            }
                            if i < chars.len() {
                                advance(&mut i, &mut line, &mut col, '}');
                            }
                            let ch = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32);
                            match ch {
                                Some(ch) => content.push(ch),
                                None => {
                                    return Err(Diagnostic::error(
                                        codes::E_LEX,
                                        format!("invalid unicode escape `\\u{{{hex}}}`"),
                                        loc,
                                    ))
                                }
                            }
                        }
                        other => {
                            return Err(Diagnostic::error(
                                codes::E_LEX,
                                format!("unknown string escape `\\{other}`"),
                                loc,
                            ))
                        }
                    }
                    continue;
                }
                content.push(c);
                advance(&mut i, &mut line, &mut col, c);
            }
            let text: String = chars[start..i].iter().collect();
            if !closed {
                return Err(Diagnostic::error(
                    codes::E_LEX,
                    "unterminated string literal",
                    Location {
                        file: file.to_string(),
                        line: tline,
                        column: tcol,
                        token: text,
                    },
                ));
            }
            toks.push(Token {
                kind: TokKind::Str(content),
                text,
                line: tline,
                column: tcol,
            });
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            {
                let ch = chars[i];
                advance(&mut i, &mut line, &mut col, ch);
            }
        }
        toks.push(Token {
            kind: TokKind::Word,
            text: chars[start..i].iter().collect(),
            line: tline,
            column: tcol,
        });
    }
    Ok(toks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_words_strings_comments() {
        let t = lex("t", ": f ( -- str ) \"a b\\n\" ; # comment\n2dup").unwrap();
        let texts: Vec<_> = t.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(
            texts,
            [":", "f", "(", "--", "str", ")", "\"a b\\n\"", ";", "2dup"]
        );
        assert_eq!(t[6].kind, TokKind::Str("a b\n".into()));
        assert_eq!(t[8].line, 2);
    }

    #[test]
    fn carriage_return_escape() {
        let t = lex("t", "\"a\\r\\n\"").unwrap();
        assert_eq!(t[0].kind, TokKind::Str("a\r\n".into()));
    }

    #[test]
    fn unicode_escape() {
        let t = lex("t", "\"\\u{1F600}\"").unwrap();
        assert_eq!(t[0].kind, TokKind::Str("\u{1F600}".into()));
    }
}
