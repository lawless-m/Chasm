#!/usr/bin/env python3
"""Build the documentation site into tmp/site/ (never committed).

Pages: index.html from site/index.md, reference.html from docs/reference.md,
tour/index.html and one tour/<name>.html per docs/tour/*.md, words.html from
the compiler's own lists (`chasm prims`, `chasm words`), and the browser
REPL copied from web/ to repl/. Every page is poured into
site/template.html; site/style.css is copied alongside.

Fenced code blocks follow the convention in site/check.py and are rendered
by `render_code`, not by the Markdown library.

Usage: python3 site/build.py     (from the repository root; needs `markdown`)
"""

import base64
import glob
import html
import json
import os
import re
import shutil
import subprocess
import sys

import markdown

from highlight import highlight, highlight_repl

OUT = "tmp/site"
GITHUB = "https://github.com/lawless-m/Chasm"
CHASM = os.environ.get("CHASM", "target/release/chasm")
KINDS = {"chasm": "chasm", "chasm fragment": "fragment", "chasm-repl": "repl"}
# Internal names that must never be published, split so this file passes its own scan.
FORBIDDEN = ("rams" + "den", "vs" + "prod")
BUILD = "RUSTUP_TOOLCHAIN=1.99.0 cargo build --release -p chasm-cli"
EMPTY = "tmp/site-build/empty.chasm"
REPL = "index.html main.js compiler.js driver.js ring.js worker.js worker-core.js coi.js coi-sw.js chasm_web.wasm".split()

# The word index: (anchor, heading) in page order. A word goes by the part
# of its name before the first `.`; names without one go by these lists.
GROUPS = [("stack", "Stack"), ("control", "Control")]
GROUPS += [(p, p) for p in ("i32", "i64", "f32", "f64", "memory", "str", "bytes", "array", "host")]
GROUPS += [("console", "Console and files"), ("option", "option"), ("vec", "vec"), ("map", "map")]
GROUPS += [("implementation", "Implementation"), ("other", "Other")]
UNDOTTED = {
    "stack": "dup drop swap over nip tuck rot -rot 2dup 2drop".split(),
    "control": "if when unless while until times leave call match trap eq hash".split(),
    "console": "print println read-line read-file write-file copy ls now".split(),
}
IMPLEMENTATION = {"vec.new", "map.new", "map.find", "map.put", "map.rehash"}


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


def render_code(info, text, root):
    """The HTML for one fenced block; `root` is the page's path to the site root."""
    kind = KINDS.get(info, "other")
    if kind == "other":
        return f'<pre data-kind="other"><code>{html.escape(text)}</code></pre>'
    body = highlight_repl(text) if kind == "repl" else highlight(text)
    link = ""
    if kind == "chasm":
        b64 = base64.urlsafe_b64encode(text.encode()).rstrip(b"=").decode()
        link = f'<a class="try" href="{root}repl/#code={b64}">Try it</a>'
    return f'<div class="code" data-kind="{kind}">{link}<pre><code>{body}</code></pre></div>'


def cut_code(text, root):
    """Replace each fenced block by a placeholder line; return text and blocks."""
    lines, blocks, fence = [], {}, None
    for line in text.split("\n"):
        stripped = line.lstrip(" ")
        indent = len(line) - len(stripped)
        if fence is None and stripped.startswith("```"):
            fence = (indent, stripped[3:].strip(), [])
        elif fence is not None and stripped.rstrip() == "```":
            indent, info, body = fence
            token = f"CHASMCODE{len(blocks)}X"
            blocks[token] = render_code(info, "\n".join(body) + "\n", root)
            lines.append(" " * indent + token)
            fence = None
        elif fence is not None:
            fence[2].append(line[min(indent, fence[0]) :])
        else:
            lines.append(line)
    return "\n".join(lines), blocks


def render(text, root):
    """Markdown to HTML. Returns the HTML and the heading tree."""
    text, blocks = cut_code(text, root)
    md = markdown.Markdown(extensions=["tables", "toc"])
    body = md.convert(text)
    for token, block in blocks.items():
        body = body.replace(f"<p>{token}</p>", block).replace(token, block)
    # Links between Markdown files become links between pages.
    body = re.sub(r'href="(?!https?:)([^"#]*)\.md(#[^"]*)?"', r'href="\1.html\2"', body)
    body = body.replace("<table>", '<div class="table"><table>').replace("</table>", "</table></div>")
    return body, md.toc_tokens


def title_of(text, fallback):
    m = re.search(r"^# (.+)$", text, re.M)
    return m.group(1).strip() if m else fallback


def compiler(*args):
    """The `results` of one `chasm ... --json` report."""
    if not os.path.isfile(CHASM):
        sys.exit(f"{CHASM} not found: run `{BUILD}`")
    run = subprocess.run([CHASM, *args, "--json"], capture_output=True, text=True)
    if run.returncode != 0:
        sys.exit(f"`{CHASM} {' '.join(args)}` failed:\n{run.stdout}{run.stderr}")
    return json.loads(run.stdout)["results"]


def group_of(word):
    name, prelude = word["name"], word["kind"] != "primitive"
    generated = word.get("generated") and not name.startswith("option.")
    if prelude and (generated or name in IMPLEMENTATION or name.startswith("chunk.")):
        return "implementation"
    if "." not in name:
        return next((group for group, names in UNDOTTED.items() if name in names), "other")
    prefix = name.split(".")[0]
    prefix = "memory" if prefix == "mem" else prefix
    return prefix if prefix in dict(GROUPS) and prefix not in UNDOTTED else "other"


def words_page():
    """The word index, from the compiler. Returns the HTML and the groups shown."""
    write(EMPTY, "")
    words = [dict(p, kind="primitive") for p in compiler("prims")["primitives"]]
    for w in compiler("words", EMPTY)["words"]:
        if w["library"]:
            words.append(dict(w, kind="prelude generic" if w["generic"] else "prelude"))
    body = [
        "<h1>Words</h1>",
        "<p>Every primitive and prelude word with its effect, generated from the compiler: the effects are "
        'as <code>chasm prims</code> and <code>chasm words</code> print them. The <a href="reference.html">'
        "reference</a> explains what the words do.</p>",
    ]
    shown = []
    for group, heading in GROUPS:
        rows = [w for w in words if group_of(w) == group]
        if not rows:
            continue
        shown.append((f"g-{group}", heading))
        body.append(f'<h2 id="g-{group}">{heading}</h2>')
        body.append('<div class="table"><table class="words">')
        body.append("<tr><th>Word</th><th>Effect</th><th>Kind</th></tr>")
        for w in rows:
            name = html.escape(w["name"])
            body.append(f'<tr><td id="w-{name}">{name}</td><td>{highlight(w["effect"])}</td><td>{w["kind"]}</td></tr>')
        body.append("</table></div>")
    return "\n".join(body), shown


def nav(root, current, tour, sections, groups):
    """The sidebar: `tour` is (name, title) pairs, the others (id, title) pairs."""

    def link(href, label, page=None):
        here = ' aria-current="page"' if page is not None and page == current else ""
        return f'<a href="{href}"{here}>{label}</a>'

    def sub(items):
        return "<ul>" + "".join(f"<li>{i}</li>" for i in items) + "</ul>" if items else ""

    tour_links = [link(f"{root}tour/{n}.html", html.escape(t), f"tour/{n}.html") for n, t in tour]
    section_links = [link(f"{root}reference.html#{i}", t) for i, t in sections]
    group_links = [link(f"{root}words.html#{i}", t) for i, t in groups]
    items = [
        link(f"{root}tour/index.html", "Tour", "tour/index.html") + sub(tour_links),
        link(f"{root}reference.html", "Reference", "reference.html") + sub(section_links),
        link(f"{root}words.html", "Words", "words.html") + sub(group_links),
        link(f"{root}repl/", "REPL"),
        link(GITHUB, "GitHub"),
    ]
    return sub(items)


def copy_repl():
    """Publish the browser REPL (web/) under repl/."""
    if not os.path.isfile("web/chasm_web.wasm"):
        sys.exit("web/chasm_web.wasm not found: run `RUSTUP_TOOLCHAIN=1.99.0 sh web/build.sh`")
    os.makedirs(f"{OUT}/repl")
    for name in REPL:
        shutil.copy(f"web/{name}", f"{OUT}/repl/{name}")


def hygiene():
    """Fail on anything that must not be published."""
    bad = []
    for folder, _, names in os.walk(OUT):
        for name in names:
            path = os.path.join(folder, name)
            if not name.endswith((".html", ".css", ".js", ".md", ".txt")):
                continue
            text = read(path).lower()
            bad += [f"{path}: contains `{word}`" for word in FORBIDDEN if word in text]
    return bad


def main():
    shutil.rmtree(OUT, ignore_errors=True)
    template = read("site/template.html")
    tour_files = sorted(glob.glob("docs/tour/*.md"))
    tour = [(os.path.basename(f)[:-3], title_of(read(f), os.path.basename(f)[:-3])) for f in tour_files]

    # (output path, title, body class, Markdown text)
    pages = [
        ("index.html", "Chasm", "landing", read("site/index.md")),
        ("reference.html", None, "", read("docs/reference.md")),
    ]
    listing = "".join(f"1. [{title}]({name}.html)\n" for name, title in tour)
    pages.append(("tour/index.html", "Tour", "", "# Tour\n\n" + (listing or "The tour has no pages yet.\n")))
    pages += [(f"tour/{name}.html", title, "", read(f)) for f, (name, title) in zip(tour_files, tour)]

    rendered, sections = [], []
    for path, title, cls, text in pages:
        root = "../" * path.count("/")
        body, toc = render(text, root)
        if path == "reference.html":
            sections = [(s["id"], s["name"]) for top in toc for s in top["children"]]
        rendered.append((path, title or title_of(text, path), cls, root, body))

    words, groups = words_page()
    rendered.append(("words.html", "Words", "", "", words))

    for path, title, cls, root, body in rendered:
        page = template
        fields = {
            "{title}": html.escape(title if path == "index.html" else f"{title} · Chasm"),
            "{class}": cls,
            "{nav}": nav(root, path, tour, sections, groups),
            "{root}": root,
            "{content}": body,
        }
        for key, value in fields.items():
            page = page.replace(key, value)
        write(f"{OUT}/{path}", page)
    shutil.copy("site/style.css", f"{OUT}/style.css")
    copy_repl()

    bad = hygiene()
    for line in bad:
        print(line, file=sys.stderr)
    if bad:
        return 1
    print(f"{OUT}: {len(rendered)} pages")
    return 0


if __name__ == "__main__":
    sys.exit(main())
