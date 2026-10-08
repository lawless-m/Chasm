"""Run the decoder over corpus files, classify each outcome, and compare
decoded images with the oracle byte for byte.

Usage: python3 tools/harness.py [--wack PATH] [--timeout SECONDS] [--jobs N]
                                [--cpu SECONDS] [--memory BYTES] [--out DIR]
                                [--nosmooth]
                                [--expect any|not-yet|refused:CODE|decoded] PATH...

Each PATH is a file or a directory; directories are walked recursively and
every regular file is a case, except names ending in .md or .txt.

Protocol: the decoder runs as `prlimit --cpu=S:S+1 --data=BYTES wack run
<FILES>` from the project root, with
two lines on stdin, `/file<input>` and `/file<output>`, both absolute, and
with --nosmooth a third, `nosmooth`, which selects plain upsampling. The
output for a case is <out>/<path from the project, '/' as '_'>.pnm (--out,
default ../tmp/harness/out); a stale one is deleted first. Outcomes are classified by text, not
exit status:

- hang:     the run exceeded the timeout
- refused:  stderr contains `REFUSED <CODE>:`
- cpu:      the decoder was killed by SIGXCPU: it used the CPU budget
- memory:   stderr says `out of memory` (wasm linear memory or the GC heap)
            or `memory allocation of` (the host's allocator): the memory
            cap was hit after the decoder's own budget check passed
- trap:     otherwise, stderr contains `trap in` or the exit status is
            non-zero (a trap without REFUSED is a decoder bug)
- not-yet:  exit 0 and the last stdout line starts `NOT_YET` (the rest is
            the reason)
- decoded:  the last stdout line is `DECODED` and the output file is
            non-empty, and it is byte-identical to the oracle's, written
            beside it as .oracle.pnm: `djpeg -dct int -pnm` (fancy
            upsampling, the default), or `djpeg -dct int -nosmooth -pnm`
            with --nosmooth
- mismatch: decoded, but the oracle failed or its output differs (the note
            gives the first differing byte, row, column and component, and for
            two-byte samples which byte)
- odd:      anything else, including an output file without DECODED

Limits, through util-linux prlimit (setrlimit in a preexec_fn is unsafe in
this threaded program):

- --cpu SECONDS (default 30; 0 disables): RLIMIT_CPU. Any file over it is
  unbounded or amplified work. The worst legal decode within the limits, a
  12-bit 4-component 4:4:4 arithmetic file with an 80-megapixel header and
  a few KB of data (both decoders feed zeros past the end), is about 18 s
  on this 40-core machine; every corpus file takes under a second.
- --memory BYTES (default 1128000000; 0 disables): RLIMIT_DATA, which
  counts committed private writable memory: the wasm linear memory, the GC
  heap and the host's allocations. It is MEM_BUDGET (1 GB) plus 128 MB of
  host headroom; RLIMIT_AS is unusable, because wasmtime reserves about
  8 GB of address space per run.
- --timeout SECONDS (default 60): the wall-clock last resort for I/O
  stalls. It exceeds the CPU budget so that CPU exhaustion classifies as
  cpu, not hang.

The exit status is non-zero if any case is trap, hang, cpu, memory, odd or
mismatch; with
`--expect not-yet`, `--expect decoded` or `--expect refused:CODE` also if
any case is not of that class (and code).
"""

import argparse
import concurrent.futures
import os
import re
import signal
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FILES = [
    "jpeg/limits.wack",
    "jpeg/refuse.wack",
    "jpeg/fixtures.wack",
    "jpeg/source.wack",
    "jpeg/frame.wack",
    "jpeg/coeffs.wack",
    "jpeg/markers.wack",
    "jpeg/bits.wack",
    "jpeg/huffman.wack",
    "jpeg/idct.wack",
    "jpeg/scan.wack",
    "jpeg/arith.wack",
    "jpeg/progressive.wack",
    "jpeg/upsample.wack",
    "jpeg/colour.wack",
    "jpeg/ppm.wack",
    "jpeg/rows.wack",
    "main.wack",
]
REFUSED = re.compile(r"REFUSED ([A-Z_]+):")
BAD = {"trap", "hang", "cpu", "memory", "odd", "mismatch"}
OUT = os.path.join(ROOT, "..", "tmp", "harness", "out")
PRLIMIT = "/usr/bin/prlimit"
CPU = 30
MEMORY = 1128000000
DJPEG = os.path.join(ROOT, "oracle", "libjpeg-turbo-3.2.0", "bin", "djpeg")


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


def oracle(src, dst, timeout, nosmooth=False):
    """Decode src with djpeg into dst: (True, note) when it wrote an image
    (exit 0, or 2 for warnings), else (False, its first stderr line)."""
    if os.path.exists(dst):
        os.remove(dst)
    try:
        r = subprocess.run(
            [DJPEG, "-dct", "int", *(["-nosmooth"] if nosmooth else []), "-pnm", "-outfile", dst, src],
            capture_output=True,
            text=True,
            errors="replace",
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return False, "oracle timed out"
    first = r.stderr.strip().splitlines()[0] if r.stderr.strip() else ""
    if r.returncode in (0, 2) and os.path.isfile(dst) and os.path.getsize(dst) > 0:
        return True, first
    return False, first or "no output"


def compare(ours, theirs):
    """None when the files are identical, else where they first differ."""
    a = open(ours, "rb").read()
    b = open(theirs, "rb").read()
    if a == b:
        return None
    if len(a) != len(b):
        return f"size {len(a)} vs {len(b)}"
    k = next(i for i in range(len(a)) if a[i] != b[i])
    fields, head = [], 0
    while len(fields) < 4 and head < len(b):
        while head < len(b) and b[head : head + 1].isspace():
            head += 1
        end = head
        while end < len(b) and not b[end : end + 1].isspace():
            end += 1
        fields.append(b[head:end])
        head = end
    head += 1  # one whitespace byte ends the header; samples may start with 0A or 20
    note = f"byte {k}"
    if len(fields) == 4 and k >= head:
        ncomp = 1 if fields[0] == b"P5" else 3
        width = int(fields[1])
        bps = 2 if int(fields[3]) > 255 else 1
        sample = (k - head) // bps
        pix = sample // ncomp
        note += f" row {pix // width} col {pix % width} comp {sample % ncomp}"
        if bps == 2:
            note += " (high byte)" if (k - head) % 2 == 0 else " (low byte)"
    return note


def limits(cpu, memory):
    """The prlimit wrapper: CPU seconds (SIGXCPU at the soft limit) and
    RLIMIT_DATA bytes; 0 leaves either out."""
    if not cpu and not memory:
        return []
    if not os.path.isfile(PRLIMIT):
        sys.exit(f"harness: {PRLIMIT} is missing (util-linux); it applies the CPU budget and memory cap")
    out = [PRLIMIT]
    if cpu:
        out.append(f"--cpu={cpu}:{cpu + 1}")
    if memory:
        out.append(f"--data={memory}")
    return out


def run(wack, timeout, path, nosmooth=False, cpu=CPU, memory=MEMORY, out=OUT):
    os.makedirs(out, exist_ok=True)
    out = os.path.join(out, os.path.relpath(path, ROOT).replace("/", "_") + ".pnm")
    if os.path.exists(out):
        os.remove(out)
    try:
        r = subprocess.run(
            [*limits(cpu, memory), wack, "run", *FILES],
            cwd=ROOT,
            input="/file" + path + "\n/file" + out + "\n" + ("nosmooth\n" if nosmooth else ""),
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
    if r.returncode == -signal.SIGXCPU:
        return "cpu", "-", first or "CPU budget exhausted"
    if "out of memory" in r.stderr or "memory allocation of" in r.stderr:
        return "memory", "-", first
    if "trap in" in r.stderr or r.returncode != 0:
        return "trap", "-", first
    lines = r.stdout.strip().splitlines()
    last = lines[-1] if lines else ""
    wrote = os.path.isfile(out) and os.path.getsize(out) > 0
    if last.startswith("NOT_YET") and not os.path.exists(out):
        return "not-yet", last[7:].strip() or "-", first
    if last == "DECODED" and wrote:
        theirs = out[: -len(".pnm")] + ".oracle.pnm"
        ok, note = oracle(path, theirs, timeout, nosmooth)
        if not ok:
            return "mismatch", "-", "oracle failed: " + note
        diff = compare(out, theirs)
        if diff:
            return "mismatch", "-", diff
        return "decoded", "-", first
    return "odd", "-", first or last or "no output"


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--wack", default=os.path.join(ROOT, "..", "target", "release", "wack"))
    ap.add_argument("--timeout", type=float, default=60)
    ap.add_argument("--cpu", type=int, default=CPU)
    ap.add_argument("--memory", type=int, default=MEMORY)
    ap.add_argument("--out", default=OUT)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--nosmooth", action="store_true")
    ap.add_argument("--expect", default="any")
    ap.add_argument("paths", nargs="+")
    a = ap.parse_args()
    if not os.path.isfile(a.wack):
        sys.exit(f"harness: {a.wack} is missing; run `cargo build --release -p wack-cli` in the Whackford repo")
    if a.expect not in ("any", "not-yet", "decoded") and not a.expect.startswith("refused:"):
        sys.exit(f"harness: unknown --expect {a.expect}")
    files = cases(a.paths)
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        results = list(pool.map(
                lambda f: run(a.wack, a.timeout, f, a.nosmooth, a.cpu, a.memory, os.path.abspath(a.out)), files
            ))

    failed = 0
    groups = {}
    for f, (cls, code, msg) in zip(files, results):
        rel = os.path.relpath(f, ROOT)
        print(f"{cls:8} {code:20} {rel}" + (f"  {msg}" if cls in BAD else ""))
        groups.setdefault(os.path.dirname(rel), []).append((cls, code))
        if cls in BAD:
            failed += 1
        elif a.expect in ("not-yet", "decoded") and cls != a.expect:
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
