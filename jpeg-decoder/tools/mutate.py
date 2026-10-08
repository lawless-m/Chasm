"""The mutation fuzzer: decode random mutants of the corpus until a time or
case count runs out, and save every input the decoder mishandles.

Usage: python3 tools/mutate.py (--minutes M | --iterations N) [--jobs N]
                               [--seed S] [--save DIR] [--work DIR]
                               [--wack PATH] [--cpu SECONDS]
                               [--memory BYTES] [--timeout SECONDS]

Sources are every regular file under corpus/ except names ending .md or
.txt, so the fuzz corpus, the generated mutants and corpus/regressions/ are
sources too. Each of the --jobs worker threads has its own
random.Random(seed * 1000 + k) and its own mutant file <work>/w<k>.jpg. A
case picks a source, applies 1 to 4 operators drawn with replacement, picks
fancy or nosmooth upsampling, and runs the decoder through harness.run (the
same protocol, oracle and classes as tools/harness.py). Operators:

- flip      1 to 8 bytes XORed with a random non-zero byte
- ff, zero  a run of 1 to 32 bytes set to FF or 00
- trunc     the file cut at a random offset (at least 1 byte kept)
- dupseg    a segment from SOI to the first SOS duplicated in place
- delseg    such a segment deleted
- swapseg   two such segments exchanged
- seglen    a segment's length set to 0, 1, 2, 3, FFFF or a random value
- splice    the file cut at a random offset, then the tail of another
            source from a random offset
- dht       a DHT's 16 counts replaced by random bytes, or its first count
            set to 3 (over-subscribed)
- sofdim    the first SOF's height and/or width set from 0, 1, 7, 8, 16,
            17, 4882, 16384, 16385, 65535 or a random value (4882 x 16384
            is just under MAX_PIXELS)
- chunk     1 to 64 random bytes overwritten
- ins       1 to 64 random bytes inserted

An operator that does not apply (too few segments, no DHT, no SOF) is
drawn again. Acceptable outcomes: decoded byte-exact against djpeg, or
refused with a code. A mismatch is a correctness finding, saved under
../tmp/mutate/mismatch/; a trap, hang, cpu, memory or odd case is a
robustness finding, saved under --save (default corpus/regressions) as
<source stem>-<kinds joined by '-'>-<first 8 hex digits of its SHA-1>.jpg
with the .txt note corpus/regressions/README.md describes, without an
`expect:` line until it is fixed. An input already saved is counted again
but not rewritten.

Limits, passed to harness.run: --cpu 30 s per decode, 1.6x the worst legal
80-megapixel decode (tmp/m8/budget-notes.txt); --memory 1128000000,
MEM_BUDGET plus 128 MB of host headroom; --timeout 60, twice the CPU
budget, so CPU exhaustion is classified cpu, not hang. At the cap 40
workers could commit about 45 GB of RAM together, which this 62 GB machine
holds. Outputs go to --work (default /dev/shm/wack-mutate/<pid>, a tmpfs,
because an 80-megapixel decode writes a 240 MB PPM and the oracle another)
and are deleted after each case; the directory is removed at exit.

Output: the seed and settings, every finding as it happens, a status line
every 60 seconds (with the slowest case so far), and last a summary line
`== N cases in M min (R/s): decoded ..., refused ..., ...; failing F`.
The exit status is 0 only when failing is 0.

The plan's week-long run: `python3 tools/mutate.py --minutes 10080` in a
tmux session.
"""

import argparse
import datetime
import hashlib
import os
import random
import shutil
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness  # noqa: E402

ROOT = harness.ROOT
MISMATCH = os.path.join(ROOT, "..", "tmp", "mutate", "mismatch")
CLASSES = ["decoded", "refused", "mismatch", "trap", "hang", "cpu", "memory", "odd"]
ROBUST = {"trap", "hang", "cpu", "memory", "odd"}
SOF = {0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF}
DIMS = [0, 1, 7, 8, 16, 17, 4882, 16384, 16385, 65535]


def segments(d):
    """(marker, offset of FF, length) for each segment from SOI to SOS."""
    out = []
    i = 2
    while i + 4 <= len(d):
        if d[i] != 0xFF:
            break
        m = d[i + 1]
        n = int.from_bytes(d[i + 2 : i + 4], "big")
        out.append((m, i, n))
        if m == 0xDA:
            break
        i += 2 + n
    return out


def whole(d, segs):
    """The segments before SOS that lie wholly inside the file."""
    return [s for s in segs if s[0] != 0xDA and s[1] + 2 + s[2] <= len(d)]


def op_flip(d, rng, srcs):
    n = rng.randint(1, 8)
    at = [rng.randrange(len(d)) for _ in range(n)]
    for i in at:
        d[i] ^= rng.randint(1, 255)
    return d, f"flip at {at}"


def op_run(value):
    def op(d, rng, srcs):
        n = rng.randint(1, 32)
        i = rng.randrange(len(d))
        d[i : i + n] = bytes([value]) * len(d[i : i + n])
        return d, f"{'ff' if value else 'zero'} {n} at {i}"

    return op


def op_trunc(d, rng, srcs):
    if len(d) < 2:
        return None
    i = rng.randint(1, len(d) - 1)
    return d[:i], f"trunc at {i}"


def op_dupseg(d, rng, srcs):
    segs = whole(d, segments(d))
    if not segs:
        return None
    m, i, n = rng.choice(segs)
    return d[: i + 2 + n] + d[i : i + 2 + n] + d[i + 2 + n :], f"dupseg {m:02X} at {i}"


def op_delseg(d, rng, srcs):
    segs = whole(d, segments(d))
    if not segs:
        return None
    m, i, n = rng.choice(segs)
    return d[:i] + d[i + 2 + n :], f"delseg {m:02X} at {i}"


def op_swapseg(d, rng, srcs):
    segs = whole(d, segments(d))
    if len(segs) < 2:
        return None
    a, b = sorted(rng.sample(segs, 2), key=lambda s: s[1])
    ea, eb = a[1] + 2 + a[2], b[1] + 2 + b[2]
    return (
        d[: a[1]] + d[b[1] : eb] + d[ea : b[1]] + d[a[1] : ea] + d[eb:],
        f"swapseg {a[0]:02X} at {a[1]} {b[0]:02X} at {b[1]}",
    )


def op_seglen(d, rng, srcs):
    segs = [s for s in segments(d) if s[1] + 4 <= len(d)]
    if not segs:
        return None
    m, i, n = rng.choice(segs)
    v = rng.choice([0, 1, 2, 3, 0xFFFF, rng.randrange(0x10000)])
    d[i + 2 : i + 4] = v.to_bytes(2, "big")
    return d, f"seglen {m:02X} at {i} to {v}"


def op_splice(d, rng, srcs):
    other = rng.choice(srcs)
    t = open(other, "rb").read()
    if not t:
        return None
    i = rng.randrange(len(d) + 1)
    j = rng.randrange(len(t))
    return d[:i] + t[j:], f"splice at {i} with {os.path.relpath(other, ROOT)} from {j}"


def op_dht(d, rng, srcs):
    segs = [s for s in whole(d, segments(d)) if s[0] == 0xC4 and s[2] >= 19]
    if not segs:
        return None
    m, i, n = rng.choice(segs)
    if rng.random() < 0.5:
        d[i + 5 : i + 21] = bytes(rng.randrange(256) for _ in range(16))
        return d, f"dht at {i} counts random"
    d[i + 5] = 3
    return d, f"dht at {i} first count 3"


def op_sofdim(d, rng, srcs):
    segs = [s for s in segments(d) if s[0] in SOF and s[1] + 9 <= len(d)]
    if not segs:
        return None
    m, i, n = segs[0]
    o = i + 4
    which = rng.choice(["height", "width", "both"])
    note = []
    if which in ("height", "both"):
        h = rng.choice(DIMS + [rng.randrange(0x10000)])
        d[o + 1 : o + 3] = h.to_bytes(2, "big")
        note.append(f"height {h}")
    if which in ("width", "both"):
        w = rng.choice(DIMS + [rng.randrange(0x10000)])
        d[o + 3 : o + 5] = w.to_bytes(2, "big")
        note.append(f"width {w}")
    return d, f"sofdim {m:02X} " + " ".join(note)


def op_chunk(d, rng, srcs):
    n = rng.randint(1, 64)
    i = rng.randrange(len(d))
    d[i : i + n] = bytes(rng.randrange(256) for _ in range(len(d[i : i + n])))
    return d, f"chunk {n} at {i}"


def op_ins(d, rng, srcs):
    n = rng.randint(1, 64)
    i = rng.randrange(len(d) + 1)
    return d[:i] + bytes(rng.randrange(256) for _ in range(n)) + d[i:], f"ins {n} at {i}"


OPS = {
    "flip": op_flip,
    "ff": op_run(0xFF),
    "zero": op_run(0x00),
    "trunc": op_trunc,
    "dupseg": op_dupseg,
    "delseg": op_delseg,
    "swapseg": op_swapseg,
    "seglen": op_seglen,
    "splice": op_splice,
    "dht": op_dht,
    "sofdim": op_sofdim,
    "chunk": op_chunk,
    "ins": op_ins,
}


def mutate(d, rng, srcs):
    """Apply 1 to 4 operators: (data, kinds, notes)."""
    kinds, notes = [], []
    want = rng.randint(1, 4)
    tries = 0
    while len(kinds) < want and tries < 40:
        tries += 1
        if not d:
            break
        k = rng.choice(list(OPS))
        r = OPS[k](bytearray(d), rng, srcs)
        if r is None:
            continue
        d, note = r
        kinds.append(k)
        notes.append(note)
    return bytes(d), kinds, notes


def sources():
    out = []
    for d, dirs, names in os.walk(os.path.join(ROOT, "corpus")):
        dirs.sort()
        for n in sorted(names):
            if not n.endswith((".md", ".txt")):
                p = os.path.join(d, n)
                if os.path.isfile(p) and os.path.getsize(p) > 0:
                    out.append(p)
    return out


class Fuzz:
    def __init__(self, a, srcs):
        self.a = a
        self.srcs = srcs
        self.lock = threading.Lock()
        self.cases = 0
        self.counts = {c: 0 for c in CLASSES}
        self.saved = set()
        self.slowest = (0.0, "")
        self.start = time.time()
        self.deadline = self.start + a.minutes * 60 if a.minutes else None

    def claim(self):
        with self.lock:
            if self.a.iterations is not None and self.cases >= self.a.iterations:
                return None
            if self.deadline is not None and time.time() >= self.deadline:
                return None
            self.cases += 1
            return self.cases

    def save(self, data, cls, src, kinds, notes, mode, msg, k, case):
        folder = MISMATCH if cls == "mismatch" else self.a.save
        sha = hashlib.sha1(data).hexdigest()
        stem = os.path.splitext(os.path.basename(src))[0]
        name = f"{stem}-{'-'.join(kinds)}-{sha[:8]}"
        with self.lock:
            if (folder, sha) in self.saved:
                return None
            self.saved.add((folder, sha))
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, name + ".jpg")
        if os.path.exists(path):
            return None
        with open(path, "wb") as f:
            f.write(data)
        lines = [
            f"class: {cls}",
            f"source: {os.path.relpath(src, ROOT)}",
            f"mutations: {'; '.join(notes)}",
            f"mode: {mode}",
            f"stderr: {msg if cls != 'mismatch' else ''}",
        ]
        if cls == "mismatch":
            lines.append(f"diff: {msg}")
        lines.append(f"found: {datetime.date.today()} tools/mutate.py --seed {self.a.seed} worker {k} case {case}")
        with open(os.path.join(folder, name + ".txt"), "w") as f:
            f.write("\n".join(lines) + "\n")
        return path

    def worker(self, k):
        rng = random.Random(self.a.seed * 1000 + k)
        path = os.path.join(self.a.work, f"w{k}.jpg")
        base = os.path.join(self.a.work, os.path.relpath(path, ROOT).replace("/", "_"))
        while True:
            case = self.claim()
            if case is None:
                return
            src = rng.choice(self.srcs)
            data, kinds, notes = mutate(open(src, "rb").read(), rng, self.srcs)
            nosmooth = rng.random() < 0.5
            with open(path, "wb") as f:
                f.write(data)
            t = time.time()
            cls, code, msg = harness.run(
                self.a.wack, self.a.timeout, path, nosmooth, self.a.cpu, self.a.memory, self.a.work
            )
            wall = time.time() - t
            for out in (base + ".pnm", base + ".oracle.pnm"):
                if os.path.exists(out):
                    os.remove(out)
            mode = "nosmooth" if nosmooth else "fancy"
            with self.lock:
                self.counts[cls] = self.counts.get(cls, 0) + 1
                if wall > self.slowest[0]:
                    self.slowest = (wall, f"{os.path.relpath(src, ROOT)} {mode} {'; '.join(notes)} -> {cls} {code}")
            if cls in ROBUST or cls == "mismatch":
                saved = self.save(data, cls, src, kinds, notes, mode, msg, k, case)
                print(f"{cls} {saved or '(already saved)'}  {msg}", flush=True)

    def status(self):
        with self.lock:
            el = time.time() - self.start
            counts = ", ".join(f"{c} {self.counts[c]}" for c in CLASSES)
            print(
                f"-- {el / 60:.1f} min, {self.cases} cases, {self.cases / max(el, 1e-9):.0f}/s: {counts}; "
                f"slowest {self.slowest[0]:.2f} s: {self.slowest[1]}",
                flush=True,
            )


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--wack", default=os.path.join(ROOT, "..", "target", "release", "wack"))
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument("--minutes", type=float)
    ap.add_argument("--iterations", type=int)
    ap.add_argument("--seed", type=int, default=int(time.time()))
    ap.add_argument("--save", default=os.path.join(ROOT, "corpus", "regressions"))
    ap.add_argument("--work")
    ap.add_argument("--cpu", type=int, default=harness.CPU)
    ap.add_argument("--memory", type=int, default=harness.MEMORY)
    ap.add_argument("--timeout", type=float, default=60)
    a = ap.parse_args()
    if a.minutes is None and a.iterations is None:
        sys.exit("mutate: give --minutes or --iterations")
    if not os.path.isfile(a.wack):
        sys.exit(f"mutate: {a.wack} is missing; run `cargo build --release -p wack-cli` in the Whackford repo")
    a.wack = os.path.abspath(a.wack)
    a.save = os.path.abspath(a.save)
    shm = os.path.isdir("/dev/shm")
    top = None
    if a.work is None:
        top = "/dev/shm/wack-mutate" if shm else os.path.join(ROOT, "..", "tmp", "mutate")
        a.work = os.path.join(top, str(os.getpid()))
    a.work = os.path.abspath(a.work)
    os.makedirs(a.work, exist_ok=True)
    srcs = sources()
    print(
        f"mutate: seed {a.seed}, {len(srcs)} sources, {a.jobs} jobs, "
        f"minutes {a.minutes}, iterations {a.iterations}, cpu {a.cpu} s, memory {a.memory}, "
        f"timeout {a.timeout} s, save {a.save}, work {a.work}",
        flush=True,
    )
    fz = Fuzz(a, srcs)
    try:
        threads = [threading.Thread(target=fz.worker, args=(k,), daemon=True) for k in range(a.jobs)]
        for t in threads:
            t.start()
        last = time.time()
        while any(t.is_alive() for t in threads):
            for t in threads:
                t.join(timeout=1)
                if time.time() - last >= 60:
                    fz.status()
                    last = time.time()
    finally:
        shutil.rmtree(a.work, ignore_errors=True)
        if top is not None:
            try:
                os.rmdir(top)
            except OSError:
                pass
    el = time.time() - fz.start
    c = fz.counts
    failing = sum(c[k] for k in ("mismatch", "trap", "hang", "cpu", "memory", "odd"))
    print(
        f"== {fz.cases} cases in {el / 60:.1f} min ({fz.cases / max(el, 1e-9):.0f}/s): "
        f"decoded {c['decoded']}, refused {c['refused']}, mismatch {c['mismatch']}, trap {c['trap']}, "
        f"hang {c['hang']}, cpu {c['cpu']}, memory {c['memory']}, odd {c['odd']}; failing {failing}"
    )
    sys.exit(1 if failing else 0)


if __name__ == "__main__":
    main()
