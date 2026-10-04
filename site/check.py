#!/usr/bin/env python3
"""Extract every ```chasm block from the docs and run it through the compiler.

Fence convention (the project convention for Markdown in this repo):

- info string exactly `chasm`: a complete program; it is checked, and the
  site gives it a Try-it button.
- info string `chasm fragment`: Chasm shown for its shape (relies on
  declarations not shown); highlighted on the site, never checked.
- info string `chasm-repl`: a REPL transcript (`> ` prompt lines and
  output); highlighted as a transcript, never checked.
- any other fence (plain, `json`, `sh`, ...): not Chasm, ignored.

Usage: python3 site/check.py [FILE.md ...]
With no arguments: docs/reference.md, docs/tour/*.md and site/index.md.
The compiler is $CHASM if set, else target/release/chasm.
"""

import glob
import os
import subprocess
import sys

BUILD = "RUSTUP_TOOLCHAIN=1.99.0 cargo build --release -p chasm-cli"
OUT_DIR = "tmp/check"
SKIPPED = ("chasm fragment", "chasm-repl")


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
    chasm = os.environ.get("CHASM", "target/release/chasm")
    if not os.path.isfile(chasm):
        print(f"{chasm} not found: run `{BUILD}`")
        return 2
    checked = skipped = failed = 0
    for path in sys.argv[1:] or default_files():
        for line, info, body in blocks(path):
            if info in SKIPPED:
                skipped += 1
                continue
            if info != "chasm":
                continue
            os.makedirs(OUT_DIR, exist_ok=True)
            stem = os.path.splitext(os.path.basename(path))[0]
            source = f"{OUT_DIR}/{stem}-{line}.chasm"
            with open(source, "w", encoding="utf-8") as f:
                f.write(body)
            run = subprocess.run([chasm, "test", source], capture_output=True, text=True)
            checked += 1
            if run.returncode != 0:
                failed += 1
                print(f"{path}:{line}: block failed")
                print(run.stdout + run.stderr, end="")
    print(f"{checked} blocks checked, {skipped} skipped")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
