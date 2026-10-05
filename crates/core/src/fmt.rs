//! `wack fmt`: one layout for Whackford source, keeping the author's line
//! breaks and the double spaces that group a line into phrases.
//!
//! - A quotation that spans lines has `[` and `]` alone on their lines (a
//!   `match` label such as `none:` keeps its `[`), its contents one level
//!   in, and what followed the `]`, the combinator, on the next line.
//!   A quotation on one line is left as it is.
//! - Items start at column 0; the rest of an item is indented 2, plus 2 per
//!   open quotation. Comment lines between items go to column 0, comment
//!   lines inside an item follow the code.
//! - Tokens are one space apart, except where the author left two or more,
//!   to group a line into phrases or to line up columns: that gap is kept.
//!   Inside `( ... )` and in a definition's header, always one. A comment
//!   after code keeps its gap.
//! - Runs of blank lines become one; none at the start or end of the file.
//!
//! The result must lex to the same tokens and comments as the input, or the
//! source is refused with `E_INTERNAL`.

use crate::diag::{codes, Diagnostic, Location};
use crate::lexer::lex;

/// A token or a comment, with the whitespace before it.
struct Piece {
    text: String,
    comment: bool,
    /// Source line of a token (for quotations that span lines).
    line: u32,
    /// Newlines and then spaces between the previous piece and this one.
    newlines: usize,
    spaces: usize,
}

/// Words that start a top-level item.
const STARTERS: [&str; 7] = [":", "export", "raw", "declare", "test", "struct", "union"];

#[derive(Clone, Copy, PartialEq)]
enum Header {
    None,
    /// After `export` or `raw`: the next word.
    Next,
    /// After `:`: the name.
    Name,
    /// After the name: a `(`.
    Paren,
}

/// Format one file's text.
pub fn format(file: &str, src: &str) -> Result<String, Diagnostic> {
    let mut pieces = pieces(file, src)?;
    break_quotations(&mut pieces);
    let out = render(&pieces);
    verify(file, src, &out)?;
    Ok(out)
}

fn pieces(file: &str, src: &str) -> Result<Vec<Piece>, Diagnostic> {
    let toks = lex(file, src)?;
    let chars: Vec<char> = src.chars().collect();
    // Char offset of each line's start; the lexer's columns count chars.
    let mut starts = vec![0usize];
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            starts.push(i + 1);
        }
    }
    let mut out = Vec::new();
    let mut at = 0usize;
    let gap = |from: usize, to: usize, out: &mut Vec<Piece>| {
        // Between tokens there is only whitespace and comments.
        let (mut newlines, mut spaces) = (0usize, 0usize);
        let mut i = from;
        while i < to {
            match chars[i] {
                '\n' => {
                    newlines += 1;
                    spaces = 0;
                    i += 1;
                }
                '#' => {
                    let end = (i..to).find(|&j| chars[j] == '\n').unwrap_or(to);
                    let text: String = chars[i..end].iter().collect();
                    out.push(Piece {
                        text: text.trim_end().to_string(),
                        comment: true,
                        line: 0,
                        newlines,
                        spaces,
                    });
                    (newlines, spaces) = (0, 0);
                    i = end;
                }
                _ => {
                    spaces += 1;
                    i += 1;
                }
            }
        }
        (newlines, spaces)
    };
    for t in &toks {
        let start = starts[t.line as usize - 1] + t.column as usize - 1;
        let (newlines, spaces) = gap(at, start, &mut out);
        at = start + t.text.chars().count();
        out.push(Piece {
            text: t.text.clone(),
            comment: false,
            line: t.line,
            newlines,
            spaces,
        });
    }
    gap(at, chars.len(), &mut out);
    Ok(out)
}

fn is_word(p: &Piece, w: &str) -> bool {
    !p.comment && p.text == w
}

/// The index of the `]` matching each `[` that closes on a later line.
fn multiline_pairs(pieces: &[Piece]) -> Vec<(usize, usize)> {
    let mut open = Vec::new();
    let mut pairs = Vec::new();
    for (i, p) in pieces.iter().enumerate() {
        if is_word(p, "[") {
            open.push(i);
        } else if is_word(p, "]") {
            if let Some(o) = open.pop() {
                if pieces[o].line != p.line {
                    pairs.push((o, i));
                }
            }
        }
    }
    pairs
}

/// A `match` arm label: `none:`, `some:`, `else:`.
fn is_label(p: &Piece) -> bool {
    !p.comment && p.text.len() > 1 && p.text.ends_with(':') && !p.text.starts_with('"')
}

fn break_before(pieces: &mut [Piece], i: usize) {
    if let Some(p) = pieces.get_mut(i) {
        if !p.comment && p.newlines == 0 {
            p.newlines = 1;
        }
    }
}

fn break_quotations(pieces: &mut [Piece]) {
    for (o, c) in multiline_pairs(pieces) {
        let labelled = o > 0 && pieces[o].newlines == 0 && is_label(&pieces[o - 1]);
        if !labelled {
            break_before(pieces, o);
        }
        break_before(pieces, o + 1);
        break_before(pieces, c);
        break_before(pieces, c + 1);
    }
}

fn render(pieces: &[Piece]) -> String {
    let multi: Vec<(usize, usize)> = multiline_pairs(pieces);
    let opens: std::collections::HashSet<usize> = multi.iter().map(|p| p.0).collect();
    let closes: std::collections::HashSet<usize> = multi.iter().map(|p| p.1).collect();
    let mut out = String::new();
    let (mut in_item, mut in_def, mut depth, mut parens) = (false, false, 0usize, 0usize);
    // A definition's header, `export raw : name (`, is one space apart.
    let mut header = Header::None;
    for (i, p) in pieces.iter().enumerate() {
        let line_start = i == 0 || p.newlines > 0;
        if line_start {
            if i > 0 {
                out.push('\n');
                if p.newlines > 1 {
                    out.push('\n');
                }
            }
            let starter = !p.comment && STARTERS.contains(&p.text.as_str());
            let indent = if depth == 0 && !in_def && starter {
                in_item = true;
                in_def = matches!(p.text.as_str(), ":" | "export" | "raw");
                0
            } else if p.comment && depth == 0 && !in_def {
                0
            } else {
                let base = if in_item { 2 } else { 0 };
                let level = if closes.contains(&i) {
                    depth - 1
                } else {
                    depth
                };
                base + 2 * level
            };
            out.push_str(&" ".repeat(indent));
        } else {
            let gap = if p.comment {
                p.spaces.max(1)
            } else if parens > 0
                || p.text == ")"
                || p.spaces < 2
                || matches!(header, Header::Next | Header::Name)
                || (header == Header::Paren && p.text == "(")
            {
                1
            } else {
                p.spaces
            };
            out.push_str(&" ".repeat(gap));
        }
        out.push_str(&p.text);
        if p.comment {
            continue;
        }
        header = match header {
            _ if in_def && line_start && matches!(p.text.as_str(), "export" | "raw") => {
                Header::Next
            }
            _ if in_def && line_start && p.text == ":" => Header::Name,
            Header::Next if p.text == ":" => Header::Name,
            Header::Next if matches!(p.text.as_str(), "export" | "raw") => Header::Next,
            Header::Next => Header::None,
            Header::Name => Header::Paren,
            _ => Header::None,
        };
        if opens.contains(&i) {
            depth += 1;
        } else if closes.contains(&i) {
            depth -= 1;
        } else if p.text == "(" {
            parens += 1;
        } else if p.text == ")" {
            parens = parens.saturating_sub(1);
        } else if p.text == ";" && depth == 0 && in_def {
            in_def = false;
            in_item = false;
        }
    }
    let mut out = out.trim().to_string();
    out.push('\n');
    if out == "\n" {
        out.clear();
    }
    out
}

/// The formatted text must hold the same tokens and comments.
fn verify(file: &str, src: &str, out: &str) -> Result<(), Diagnostic> {
    let texts = |s: &str| -> Result<Vec<(String, bool)>, Diagnostic> {
        Ok(pieces(file, s)?
            .into_iter()
            .map(|p| (p.text, p.comment))
            .collect())
    };
    let same = texts(src)? == texts(out)?
        && lex(file, src)?
            .iter()
            .map(|t| &t.kind)
            .eq(lex(file, out)?.iter().map(|t| &t.kind));
    if same {
        return Ok(());
    }
    Err(Diagnostic::error(
        codes::E_INTERNAL,
        "wack fmt would change the program's tokens; the file is left as it is",
        Location {
            file: file.to_string(),
            line: 1,
            column: 1,
            token: String::new(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::format;

    fn f(s: &str) -> String {
        format("t.wack", s).unwrap()
    }

    #[test]
    fn multiline_quotations_get_their_own_lines() {
        let src = "\
: f ( i32 -- i32 )
  :> h
  h 0 i32.lt_s
  [ h ]
  [ 64 bytes.new :> buf
    buf bytes.len ] if ;
";
        assert_eq!(
            f(src),
            "\
: f ( i32 -- i32 )
  :> h
  h 0 i32.lt_s
  [ h ]
  [
    64 bytes.new :> buf
    buf bytes.len
  ]
  if ;
"
        );
    }

    #[test]
    fn labels_keep_their_bracket_and_nesting_indents() {
        let src = "\
: f ( option i32 -- i32 )
  none: [ 0
     1 i32.add ]
  some: [ ]
  match ;
";
        assert_eq!(
            f(src),
            "\
: f ( option i32 -- i32 )
  none: [
    0
    1 i32.add
  ]
  some: [ ]
  match ;
"
        );
    }

    #[test]
    fn spacing_comments_and_blank_lines() {
        let src = "\n\n#  a comment   \n:  sq   (  i32 --   i32 )  dup    i32.mul ;   # square\n\n\n\n    test sq : 3 sq -> 9\nstruct p  x: i32\n   y: i32\n";
        assert_eq!(
            f(src),
            "#  a comment\n: sq ( i32 -- i32 )  dup    i32.mul ;   # square\n\ntest sq : 3 sq -> 9\nstruct p  x: i32\n  y: i32\n"
        );
    }

    #[test]
    fn headers_are_tight() {
        assert_eq!(
            f("export  raw   :  p  (  i32 -- i32 )  i32.load ;\n"),
            "export raw : p ( i32 -- i32 )  i32.load ;\n"
        );
    }

    #[test]
    fn strings_are_verbatim() {
        let src = ": s ( -- str )  \"a  b # c\n  d\" ;\n";
        assert_eq!(f(src), src);
    }

    #[test]
    fn idempotent() {
        let src = "\
: f ( -- )
  3 [ :> i
    i 0 i32.eq [ \"zero\" println ]
    [ i i32.to-str println ] if ] times ;
";
        let once = f(src);
        assert_eq!(f(&once), once);
        assert_eq!(
            once,
            "\
: f ( -- )
  3
  [
    :> i
    i 0 i32.eq [ \"zero\" println ]
    [ i i32.to-str println ] if
  ]
  times ;
"
        );
    }
}
