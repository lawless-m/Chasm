"""Run the decoder over corpus files and classify each outcome.

Usage: python3 tools/harness.py [--wack PATH] [--timeout SECONDS] [--jobs N]
                                [--expect any|not-yet|refused:CODE] PATH...

Each PATH is a file or a directory; directories are walked recursively and
every regular file is a case, except names ending in .md or .txt.

Protocol: the decoder runs as `wack run <FILES>` from the project root, with
`/file<absolute path>` and a newline on stdin. Outcomes are classified by
text, not exit status:

- hang:     the run exceeded the timeout
- refused:  stderr contains `REFUSED <CODE>:`
- trap:     otherwise, stderr contains `trap in` or the exit status is
            non-zero (a trap without REFUSED is a decoder bug)
- not-yet:  exit 0 and the last stdout line is `NOT_YET SOS`
- odd:      anything else

The exit status is non-zero if any case is trap, hang or odd; with
`--expect not-yet` also if any case is not not-yet; with
`--expect refused:CODE` also if any case is not refused with that code.
"""

import argparse
import concurrent.futures
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FILES = [
    "jpeg/limits.wack",
    "jpeg/refuse.wack",
    "jpeg/fixtures.wack",
    "jpeg/source.wack",
    "jpeg/frame.wack",
    "jpeg/markers.wack",
    "jpeg/bits.wack",
    "jpeg/huffman.wack",
    "main.wack",
]
REFUSED = re.compile(r"REFUSED ([A-Z_]+):")
BAD = {"trap", "hang", "odd"}


def cases(paths):
    out = []
    for p in paths:
        p = os.path.abspath(p)
        if os.path.isdir(p):
            for d, dirs, names in os.walk(p):
                dirs.sort()
                for n in sorted(names):
                    if not n.endswith((".md", ".txt")):
                        out.append(os.path.join(d, n))
        elif os.path.isfile(p):
            out.append(p)
        else:
            sys.exit(f"harness: no such file or directory: {p}")
    return out


def run(wack, timeout, path):
    try:
        r = subprocess.run(
            [wack, "run", *FILES],
            cwd=ROOT,
            input="/file" + path + "\n",
            capture_output=True,
            text=True,
            errors="replace",
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return "hang", "-", ""
    first = r.stderr.strip().splitlines()[0] if r.stderr.strip() else ""
    m = REFUSED.search(r.stderr)
    if m:
        return "refused", m.group(1), first
    if "trap in" in r.stderr or r.returncode != 0:
        return "trap", "-", first
    lines = r.stdout.strip().splitlines()
    if lines and lines[-1] == "NOT_YET SOS":
        return "not-yet", "-", first
    return "odd", "-", first or (lines[-1] if lines else "no output")


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--wack", default=os.path.join(ROOT, "..", "target", "release", "wack"))
    ap.add_argument("--timeout", type=float, default=20)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--expect", default="any")
    ap.add_argument("paths", nargs="+")
    a = ap.parse_args()
    if not os.path.isfile(a.wack):
        sys.exit(f"harness: {a.wack} is missing; run `cargo build --release -p wack-cli` in the Whackford repo")
    if a.expect not in ("any", "not-yet") and not a.expect.startswith("refused:"):
        sys.exit(f"harness: unknown --expect {a.expect}")
    files = cases(a.paths)
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        results = list(pool.map(lambda f: run(a.wack, a.timeout, f), files))

    failed = 0
    groups = {}
    for f, (cls, code, msg) in zip(files, results):
        rel = os.path.relpath(f, ROOT)
        print(f"{cls:8} {code:20} {rel}" + (f"  {msg}" if cls in BAD else ""))
        groups.setdefault(os.path.dirname(rel), []).append((cls, code))
        if cls in BAD:
            failed += 1
        elif a.expect == "not-yet" and cls != "not-yet":
            failed += 1
        elif a.expect.startswith("refused:") and (cls, code) != ("refused", a.expect[8:]):
            failed += 1
    for d, rs in groups.items():
        classes = {}
        codes = {}
        for cls, code in rs:
            classes[cls] = classes.get(cls, 0) + 1
            if cls == "refused":
                codes[code] = codes.get(code, 0) + 1
        line = ", ".join(f"{k} {v}" for k, v in sorted(classes.items()))
        if codes:
            line += "; " + ", ".join(f"{k} {v}" for k, v in sorted(codes.items()))
        print(f"== {d}: {len(rs)} files: {line}")
    print(f"== {len(files)} files, {failed} failing")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
