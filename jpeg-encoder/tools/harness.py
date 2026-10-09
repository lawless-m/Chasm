"""Run the encoder over PPM inputs and compare each output with cjpeg's byte
for byte.

Usage: python3 -I tools/harness.py encode [--quality 1,10,...] [--sample 420,422,444]
                                          [--wack PATH] [--jobs N] [--cpu SECONDS]
                                          [--memory BYTES] [--timeout SECONDS] [--out DIR]
                                          [--icc PATH] [--expect any|match|refused:CODE] PATH...

Each PATH is a .ppm, .pgm or .pnm file, or a directory walked recursively
(names ending in .md or .txt are skipped). A case is one input at one
quality and one sampling; a P5 input is greyscale, which ignores sampling,
so it gives one case per quality, labelled `grey`.

Protocol: the encoder runs as `prlimit --cpu=S:S+1 --data=BYTES wack run
<FILES>` from the jpeg-encoder directory, with stdin lines `/file<input>`,
`/file<output>`, `quality=Q`, `sample=S` for a P6 input, and with --icc
`icc=/file<profile>`. The output is
<out>/<input stem>-q<Q>-<S or grey>.jpg (a stale one is deleted first); the
reference beside it, .ref.jpg, is made by the oracle:

    cjpeg -dct int -baseline -quality Q -sample 2x2|2x1|1x1 [-grayscale] [-icc PROFILE] -outfile REF IN

with 420, 422, 444 as 2x2, 2x1, 1x1, and a P5 input as 1x1 with
-grayscale. -baseline clamps the scaled tables to 255, as the encoder
does: without it cjpeg writes 16-bit tables (not baseline) below about
quality 25. Outcomes are classified by text, not exit status:

- hang:     the run exceeded the timeout
- refused:  stderr contains `REFUSED <CODE>:`
- cpu:      killed by SIGXCPU: the CPU budget was used up
- memory:   stderr says `out of memory` or `memory allocation of`
- trap:     otherwise, stderr contains `trap in` or the exit status is
            non-zero (a trap without REFUSED is an encoder bug)
- match:    the last stdout line is `ENCODED <w> <h> <n>`, the output is
            non-empty, and it is byte-identical to the reference
- mismatch: encoded, but cjpeg failed or the files differ; the note gives
            `size A vs B`, or the first differing byte and the segment of
            the reference it falls in (see `locate`)
- odd:      anything else

Limits are the decoder harness's: --cpu 90 s, --memory 1128000000 bytes of
RLIMIT_DATA, --timeout 180 s wall clock.

The exit status is non-zero if any case is mismatch, trap, hang, cpu,
memory or odd; with `--expect match` also if any case is not a match, and
with `--expect refused:CODE` if any case is not that refusal.
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
    "../jpeg-decoder/jpeg/limits.wack",
    "../jpeg-decoder/jpeg/refuse.wack",
    "../jpeg-decoder/jpeg/fixtures.wack",
    "jpeg/elimits.wack",
    "jpeg/qtables.wack",
    "jpeg/htables.wack",
    "jpeg/ppmread.wack",
    "jpeg/writer.wack",
    "jpeg/bitwriter.wack",
    "jpeg/huffenc.wack",
    "jpeg/fdct.wack",
    "jpeg/quantise.wack",
    "jpeg/ccolour.wack",
    "jpeg/downsample.wack",
    "jpeg/encoder.wack",
    "encode.wack",
]
REFUSED = re.compile(r"REFUSED ([A-Z_]+):")
ENCODED = re.compile(r"^ENCODED \d+ \d+ \d+$")
BAD = {"mismatch", "trap", "hang", "cpu", "memory", "odd"}
OUT = os.path.join(ROOT, "..", "tmp", "harness-enc", "out")
PRLIMIT = "/usr/bin/prlimit"
CPU = 90
MEMORY = 1128000000
CJPEG = os.path.join(ROOT, "..", "jpeg-decoder", "oracle", "libjpeg-turbo-3.2.0", "bin", "cjpeg")
SAMPLES = {"420": "2x2", "422": "2x1", "444": "1x1"}
GRID_QUALITY = "1,10,40,50,75,85,90,95,100"
GRID_SAMPLE = "420,422,444"


def inputs(paths):
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


def locate(data, k):
    """The segment of a JPEG file holding byte offset k: `SOI`, `APPn`,
    `DQT`, `SOF0`, `DHT n` (n counts the DHT segments from 1), `SOS`, `EOI`,
    or `scan +N` for the Nth byte (from 0) of the entropy data after the SOS
    segment. Naming the MCU an entropy-data offset belongs to would need an
    entropy decoder; that is left for when a scan mismatch first appears
    (E1)."""
    names = {0xC0: "SOF0", 0xC4: "DHT", 0xDA: "SOS", 0xDB: "DQT", 0xD8: "SOI", 0xD9: "EOI"}
    i, dhts = 0, 0
    while i + 1 < len(data):
        if data[i] != 0xFF:
            return f"byte {i} outside any segment"
        m = data[i + 1]
        if m == 0xD8 or m == 0xD9:
            if k <= i + 1:
                return names[m]
            i += 2
            continue
        length = (data[i + 2] << 8) | data[i + 3]
        end = i + 2 + length
        if m == 0xC4:
            dhts += 1
        if k < end:
            if m == 0xC4:
                return f"DHT {dhts}"
            if 0xE0 <= m <= 0xEF:
                return f"APP{m - 0xE0}"
            return names.get(m, f"marker {m:02X}")
        i = end
        if m == 0xDA:
            j = i
            while j + 1 < len(data) and not (data[j] == 0xFF and data[j + 1] != 0x00 and not 0xD0 <= data[j + 1] <= 0xD7):
                j += 1
            if k < j:
                return f"scan +{k - i}"
            i = j
    return f"byte {k} past the end"


def note(ours, ref):
    a = open(ours, "rb").read()
    b = open(ref, "rb").read()
    if len(a) != len(b):
        return f"size {len(a)} vs {len(b)}"
    k = next(i for i in range(len(a)) if a[i] != b[i])
    return f"byte {k} in {locate(b, k)}"


def reference(src, dst, quality, sample, timeout, icc=None):
    """Make cjpeg's file: (True, '') when it wrote one, else (False, its
    first stderr line)."""
    if os.path.exists(dst):
        os.remove(dst)
    grey = sample == "grey"
    try:
        r = subprocess.run(
            [CJPEG, "-dct", "int", "-baseline", "-quality", str(quality), "-sample", "1x1" if grey else SAMPLES[sample],
             *(["-grayscale"] if grey else []), *(["-icc", icc] if icc else []), "-outfile", dst, src],
            capture_output=True,
            text=True,
            errors="replace",
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return False, "cjpeg timed out"
    if r.returncode != 0 or not os.path.isfile(dst):
        return False, (r.stderr.strip().splitlines() or ["no output"])[0]
    return True, ""


def run(a, case):
    path, quality, sample = case
    os.makedirs(a.out, exist_ok=True)
    stem = os.path.splitext(os.path.basename(path))[0]
    out = os.path.join(a.out, f"{stem}-q{quality}-{sample}.jpg")
    if os.path.exists(out):
        os.remove(out)
    stdin = f"/file{path}\n/file{out}\nquality={quality}\n" + ("" if sample == "grey" else f"sample={sample}\n")
    if a.icc:
        stdin += f"icc=/file{a.icc}\n"
    try:
        r = subprocess.run(
            [*limits(a.cpu, a.memory), a.wack, "run", *FILES],
            cwd=ROOT,
            input=stdin,
            capture_output=True,
            text=True,
            errors="replace",
            timeout=a.timeout,
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
    if ENCODED.match(last) and os.path.isfile(out) and os.path.getsize(out) > 0:
        ref = out[: -len(".jpg")] + ".ref.jpg"
        ok, why = reference(path, ref, quality, sample, a.timeout, a.icc)
        if not ok:
            return "mismatch", "-", "cjpeg failed: " + why
        if open(out, "rb").read() != open(ref, "rb").read():
            return "mismatch", "-", note(out, ref)
        return "match", "-", first
    return "odd", "-", first or last or "no output"


def encode(a):
    if not os.path.isfile(a.wack):
        sys.exit(f"harness: {a.wack} is missing; run `cargo build -p wack-cli` in the Whackford repo")
    if a.expect not in ("any", "match") and not a.expect.startswith("refused:"):
        sys.exit(f"harness: unknown --expect {a.expect}")
    qualities = [int(q) for q in a.quality.split(",")]
    samples = a.sample.split(",")
    for s in samples:
        if s not in SAMPLES:
            sys.exit(f"harness: unknown sample {s}")
    a.out = os.path.abspath(a.out)
    a.wack = os.path.abspath(a.wack)
    if a.icc:
        a.icc = os.path.abspath(a.icc)
    cases = []
    for p in inputs(a.paths):
        with open(p, "rb") as f:
            grey = f.read(2) == b"P5"
        for q in qualities:
            for s in ["grey"] if grey else samples:
                cases.append((p, q, s))
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        results = list(pool.map(lambda c: run(a, c), cases))
    failed = 0
    for (p, q, s), (cls, code, msg) in zip(cases, results):
        rel = os.path.relpath(p, ROOT)
        print(f"{cls:8} {code:20} {rel} q{q} {s}" + (f"  {msg}" if cls in BAD else ""))
        if cls in BAD:
            failed += 1
        elif a.expect == "match" and cls != "match":
            failed += 1
        elif a.expect.startswith("refused:") and (cls, code) != ("refused", a.expect[8:]):
            failed += 1
    print(f"== {len(cases)} cases, {failed} failing")
    sys.exit(1 if failed else 0)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    e = sub.add_parser("encode", help="PPM inputs against cjpeg")
    e.add_argument("--quality", default=GRID_QUALITY)
    e.add_argument("--sample", default=GRID_SAMPLE)
    e.add_argument("--wack", default=os.path.join(ROOT, "..", "target", "debug", "wack"))
    e.add_argument("--jobs", type=int, default=4)
    e.add_argument("--cpu", type=int, default=CPU)
    e.add_argument("--memory", type=int, default=MEMORY)
    e.add_argument("--timeout", type=float, default=180)
    e.add_argument("--out", default=OUT)
    e.add_argument("--icc", default=None)
    e.add_argument("--expect", default="any")
    e.add_argument("paths", nargs="+")
    e.set_defaults(fn=encode)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
