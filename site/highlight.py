"""Syntax highlighting for Chasm code blocks on the site: text in, HTML out.

A small tokenizer over the lexical rules of docs/reference.md sections 2 to
4. Token classes: tok-c comment, tok-s string, tok-n number, tok-k keyword,
tok-e effect or stack assertion, tok-q quotation bracket, tok-l label,
tok-t type.
"""

import html
import re

KEYWORDS = {":", ";", "export", "declare", "test", "struct", "union", "->", ":>"}
TYPES = {"i32", "i64", "f32", "f64", "str", "bytes", "array", "vec", "map", "option"}
NUMBER = re.compile(r"-?(0x[0-9a-fA-F]+|\d+(\.\d+)?([eE][-+]?\d+)?)$")
TOKEN = re.compile(r"\s+|\S+")


def span(cls, text):
    return f'<span class="tok-{cls}">{html.escape(text, quote=False)}</span>'


def classify(token):
    if token in KEYWORDS:
        return "k"
    if token in ("[", "]"):
        return "q"
    if NUMBER.match(token):
        return "n"
    if token in TYPES or token[0].isupper():
        return "t"
    if token.endswith(":"):
        return "l"
    return None


def effect_end(text, start):
    """The index just past the `)` matching the `(` token at `start`."""
    depth = 0
    for m in TOKEN.finditer(text, start):
        depth += {"(": 1, ")": -1}.get(m.group(), 0)
        if depth == 0:
            return m.end()
    return len(text)


def string_end(text, start):
    """The index just past the string whose opening quote is at `start`."""
    i = start + 1
    while i < len(text) and text[i] != '"':
        i += 2 if text[i] == "\\" else 1
    return min(i + 1, len(text))


def highlight(text):
    out, i = [], 0
    while i < len(text):
        m = TOKEN.match(text, i)
        token = m.group()
        if token.isspace():
            out.append(token)
            i = m.end()
        elif token.startswith("#"):
            end = text.find("\n", i)
            end = len(text) if end == -1 else end
            out.append(span("c", text[i:end]))
            i = end
        elif token.startswith('"'):
            end = string_end(text, i)
            out.append(span("s", text[i:end]))
            i = end
        elif token == "(":
            end = effect_end(text, i)
            out.append(span("e", text[i:end]))
            i = end
        else:
            cls = classify(token)
            out.append(span(cls, token) if cls else html.escape(token, quote=False))
            i = m.end()
    return "".join(out)


def highlight_repl(text):
    """A transcript: `> ` and `. ` lines are prompt plus Chasm, the rest output."""
    lines = []
    for line in text.split("\n"):
        if line.startswith(("> ", ". ")):
            lines.append(span("c", line[:2]) + highlight(line[2:]))
        else:
            lines.append(html.escape(line, quote=False))
    return "\n".join(lines)
