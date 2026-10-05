#!/usr/bin/env python3
"""Extract every ```wack block from the docs and run it through the compiler.

Fence convention (the project convention for Markdown in this repo):

- info string exactly `wack`: a complete program; it is checked, and the
  site gives it a Try-it button.
- info string `wack fragment`: Whackford shown for its shape (relies on
  declarations not shown); highlighted on the site, never checked.
- info string `wack-repl`: a REPL transcript (`> ` prompt lines and
  output); highlighted as a transcript, never checked.
- any other fence (plain, `json`, `sh`, ...): not Whackford, ignored.

Usage: python3 site/check.py [FILE.md ...]
With no arguments: docs/reference.md, docs/tour/*.md and site/index.md.
The compiler is $WACK if set, else target/release/wack.
"""

import glob
import os
import subprocess
import sys

BUILD = "RUSTUP_TOOLCHAIN=1.99.0 cargo build --release -p wack-cli"
OUT_DIR = "tmp/check"
SKIPPED = ("wack fragment", "wack-repl")


def default_files():
    files = ["docs/reference.md"] + sorted(glob.glob("docs/tour/*.md")) + ["site/index.md"]
    return [f for f in files if os.path.isfile(f)]


def blocks(path):
    """Yield (fence line number, info string, body) for each fenced block."""
    with open(path, encoding="utf-8") as f:
        lines = f.read().split("\n")
    start = None
    for number, line in enumerate(lines, 1):
        if not line.startswith("```"):
            continue
        if start is None:
            start, info = number, line[3:].strip()
        else:
            yield start, info, "\n".join(lines[start : number - 1]) + "\n"
            start = None


def main():
    wack = os.environ.get("WACK", "target/release/wack")
    if not os.path.isfile(wack):
        print(f"{wack} not found: run `{BUILD}`")
        return 2
    checked = skipped = failed = 0
    for path in sys.argv[1:] or default_files():
        for line, info, body in blocks(path):
            if info in SKIPPED:
                skipped += 1
                continue
            if info != "wack":
                continue
            os.makedirs(OUT_DIR, exist_ok=True)
            stem = os.path.splitext(os.path.basename(path))[0]
            source = f"{OUT_DIR}/{stem}-{line}.wack"
            with open(source, "w", encoding="utf-8") as f:
                f.write(body)
            run = subprocess.run([wack, "test", source], capture_output=True, text=True)
            checked += 1
            if run.returncode != 0:
                failed += 1
                print(f"{path}:{line}: block failed")
                print(run.stdout + run.stderr, end="")
    print(f"{checked} blocks checked, {skipped} skipped")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
