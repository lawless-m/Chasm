"""The encoder's mutation fuzzer: run clean on random mutants of the decoder's
and the encoder's corpora until a time or case count runs out, and save
every input clean mishandles.

Usage: python3 -I tools/mutate_clean.py (--minutes M | --iterations N) [--jobs N]
                                        [--seed S] [--save DIR] [--work DIR]
                                        [--wack PATH] [--cpu SECONDS]
                                        [--memory BYTES] [--timeout SECONDS]
                                        [--quality 40,85] [--sample 420,422,444]

Reused, not copied: the decoder's fuzzer, ../jpeg-decoder/tools/mutate.py,
is loaded read-only (as the module dec_mutate) for its operators (mutate)
and its sources (every non-empty file under the decoder's corpus but .md
and .txt names); this directory's tools/harness.py is loaded under its own
name, enc_harness (the decoder's fuzzer registers the decoder's harness as
`harness`), for decode_phase and clean_case, so every protocol, class rule
and reference command is the clean harness's own.

Sources: the decoder's, plus every non-empty file under this directory's
corpus/ but .md and .txt names (the synthetic metadata cases and the
regressions). A case takes its source from the encoder's list with
probability 1/4 (the metadata segments are clean's own attack surface),
from the decoder's otherwise; splice draws from both.

Per case, in worker k (random.Random(seed * 1000 + k), its own directory
<work>/w<k>/, wiped after every case): the source mutated by 1 to 4 of the
decoder fuzzer's operators; a quality and a sampling drawn from --quality
and --sample; enc_harness.decode_phase (the decoder's own run with fancy
upsampling, djpeg's decode, the metadata oracle's prediction); then
enc_harness.clean_case (clean, and when it wrote an output the reference
cjpeg [-icc p] (rotate(djpeg x))), with the sampling `grey` when the
decoder's decode is P5. Classes: match; refused (agreeing with the decoder
or the oracle); disagree (a refusal agreeing with neither); mismatch,
trap, hang, cpu, memory, odd as the clean harness gives them. Failing is
everything but match and refused.

A failing case is a finding, saved into --save (default corpus/regressions)
as <source stem>-<kinds joined by '-'>-<first 8 hex digits of its SHA-1>.jpg
with a note <same>.txt, one `key: value` a line: class, code, source (the
path under the Whackford directory), mutations, quality, sample, kind (grey
or colour, the decoder's decode), decoder (its class and code), oracle
(`META O B`, `REFUSED CODE` or `UNPARSABLE`), stderr (clean's first line),
note (the harness's message), found. There is no `expect:` line until the
finding is fixed (corpus/regressions/README.md). An input already saved is
counted again but not rewritten. A mismatch also has clean's output and the
reference copied to ../tmp/e7/findings/<name>.<file>. A finding whose
`decoder:` line is itself a failing class is a decoder finding: reported,
not fixed here.

Limits, the clean harness's: --cpu 90 s and --memory 1128000000 bytes of
RLIMIT_DATA per run, as for the decoder (clean on an 80-megapixel probe
takes about 13 s of CPU and under 750 MB, ../tmp/e7/big63.txt and
../tmp/e7/big-o6.txt); --timeout 180 s wall clock. At the cap 38 workers
can commit about 43 GB together, which this 62 GB machine holds beside
the tmpfs outputs. Work goes to --work (default
/dev/shm/wack-mutate-clean/<pid>, a tmpfs: an 80-megapixel case writes a
240 MB decode, a turned copy, the output and the reference), removed at
exit.

Output: the seed and settings, every finding as it happens, a status line
every 60 seconds (with the slowest case so far), and last
`== N cases in M min (R/s): match ..., refused ..., disagree ..., mismatch
..., trap ..., hang ..., cpu ..., memory ..., odd ...; failing F`. The exit
status is 0 only when failing is 0.

The one-hour run: `python3 -I tools/mutate_clean.py --minutes 60 --jobs 38`.
"""

import argparse
import datetime
import hashlib
import importlib.util
import os
import random
import shutil
import sys
import threading
import time
import types

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TOP = os.path.dirname(ROOT)
DEC_MUTATE = os.path.join(ROOT, "..", "jpeg-decoder", "tools", "mutate.py")
ENC_HARNESS = os.path.join(ROOT, "tools", "harness.py")
FINDINGS = os.path.join(ROOT, "..", "tmp", "e7", "findings")
CLASSES = ["match", "refused", "disagree", "mismatch", "trap", "hang", "cpu", "memory", "odd"]
FAILING = {"disagree", "mismatch", "trap", "hang", "cpu", "memory", "odd"}


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


dec_mutate = load("dec_mutate", DEC_MUTATE)
enc_harness = load("enc_harness", ENC_HARNESS)


def encoder_sources():
    out = []
    for d, dirs, names in os.walk(os.path.join(ROOT, "corpus")):
        dirs.sort()
        for n in sorted(names):
            if not n.endswith((".md", ".txt")):
                p = os.path.join(d, n)
                if os.path.isfile(p) and os.path.getsize(p) > 0:
                    out.append(p)
    return out


def oracle_line(pred):
    code, orient, prof = pred
    if code:
        return f"REFUSED {code}"
    if orient is None:
        return "UNPARSABLE"
    return f"META {orient} {len(prof) if prof else 0}"


class Fuzz:
    def __init__(self, a, dsrcs, esrcs):
        self.a = a
        self.dsrcs = dsrcs
        self.esrcs = esrcs
        self.all = dsrcs + esrcs
        self.dec = enc_harness.load_decoder_harness()
        self.meta = enc_harness.load_meta_ref()
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

    def save(self, data, info, k, case, wdir, mutant):
        sha = hashlib.sha1(data).hexdigest()
        stem = os.path.splitext(os.path.basename(info["src"]))[0]
        name = f"{stem}-{'-'.join(info['kinds'])}-{sha[:8]}"
        folder = self.a.save
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
            f"class: {info['cls']}",
            f"code: {info['code']}",
            f"source: {os.path.relpath(info['src'], TOP)}",
            f"mutations: {'; '.join(info['notes'])}",
            f"quality: {info['quality']}",
            f"sample: {info['sample']}",
            f"kind: {info['kind']}",
            f"decoder: {info['dcls']} {info['dcode']}",
            f"oracle: {info['oracle']}",
            f"stderr: {info['stderr']}",
            f"note: {info['msg']}",
            f"found: {datetime.date.today()} tools/mutate_clean.py --seed {self.a.seed} worker {k} case {case}",
        ]
        with open(os.path.join(folder, name + ".txt"), "w") as f:
            f.write("\n".join(lines) + "\n")
        if info["cls"] == "mismatch":
            os.makedirs(FINDINGS, exist_ok=True)
            for n in os.listdir(wdir):
                p = os.path.join(wdir, n)
                if n.endswith(".jpg") and p != mutant and os.path.isfile(p):
                    shutil.copyfile(p, os.path.join(FINDINGS, f"{name}.{n}"))
        return path

    def worker(self, k):
        rng = random.Random(self.a.seed * 1000 + k)
        wdir = os.path.join(self.a.work, f"w{k}")
        mutant = os.path.join(wdir, f"w{k}.jpg")
        qualities = [int(q) for q in self.a.quality.split(",")]
        samples = self.a.sample.split(",")
        h = types.SimpleNamespace(wack=self.a.wack, timeout=self.a.timeout, cpu=self.a.cpu, memory=self.a.memory, out=wdir)
        while True:
            case = self.claim()
            if case is None:
                return
            shutil.rmtree(wdir, ignore_errors=True)
            os.makedirs(wdir)
            src = rng.choice(self.esrcs) if self.esrcs and rng.random() < 0.25 else rng.choice(self.dsrcs)
            data, kinds, notes = dec_mutate.mutate(open(src, "rb").read(), rng, self.all)
            quality = rng.choice(qualities)
            sample = rng.choice(samples)
            with open(mutant, "wb") as f:
                f.write(data)
            t = time.time()
            d = enc_harness.decode_phase(h, self.dec, self.meta, mutant)
            grey = d[0] == "decoded" and d[3] is not None and enc_harness.pnm_header(d[3])[0] == b"P5"
            if grey:
                sample = "grey"
            cls, code, msg, agrees, orient, icc = enc_harness.clean_case(h, self.dec, (mutant, quality, sample, d))
            wall = time.time() - t
            if cls == "refused" and not agrees:
                cls = "disagree"
            if cls not in CLASSES:
                cls = "odd"
            with self.lock:
                self.counts[cls] += 1
                if wall > self.slowest[0]:
                    self.slowest = (wall, f"{os.path.relpath(src, TOP)} q{quality} {sample} {'; '.join(notes)} -> {cls} {code}")
            if cls in FAILING:
                info = {
                    "cls": cls, "code": code if cls in ("refused", "disagree") else "-", "src": src,
                    "kinds": kinds, "notes": notes, "quality": quality, "sample": sample,
                    "kind": "grey" if grey else "colour", "dcls": d[0], "dcode": d[1],
                    "oracle": oracle_line(d[4]), "stderr": "", "msg": msg or "",
                }
                if cls in ("trap", "hang", "cpu", "memory", "odd", "disagree"):
                    info["stderr"] = msg or ""
                saved = self.save(data, info, k, case, wdir, mutant)
                print(f"{cls} {code} {saved or '(already saved)'}  {msg}", flush=True)
        shutil.rmtree(wdir, ignore_errors=True)

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
    ap.add_argument("--cpu", type=int, default=enc_harness.CPU)
    ap.add_argument("--memory", type=int, default=enc_harness.MEMORY)
    ap.add_argument("--timeout", type=float, default=180)
    ap.add_argument("--quality", default="40,85")
    ap.add_argument("--sample", default="420,422,444")
    a = ap.parse_args()
    if a.minutes is None and a.iterations is None:
        sys.exit("mutate_clean: give --minutes or --iterations")
    if not os.path.isfile(a.wack):
        sys.exit(f"mutate_clean: {a.wack} is missing; run `cargo build --release -p wack-cli` in the Whackford repo")
    a.wack = os.path.abspath(a.wack)
    a.save = os.path.abspath(a.save)
    top = None
    if a.work is None:
        top = "/dev/shm/wack-mutate-clean" if os.path.isdir("/dev/shm") else os.path.join(ROOT, "..", "tmp", "e7", "mutate")
        a.work = os.path.join(top, str(os.getpid()))
    a.work = os.path.abspath(a.work)
    os.makedirs(a.work, exist_ok=True)
    dsrcs = dec_mutate.sources()
    esrcs = encoder_sources()
    print(
        f"mutate_clean: seed {a.seed}, {len(dsrcs) + len(esrcs)} sources ({len(dsrcs)} decoder, {len(esrcs)} encoder), "
        f"{a.jobs} jobs, minutes {a.minutes}, iterations {a.iterations}, quality {a.quality}, sample {a.sample}, "
        f"cpu {a.cpu} s, memory {a.memory}, timeout {a.timeout} s, save {a.save}, work {a.work}",
        flush=True,
    )
    fz = Fuzz(a, dsrcs, esrcs)
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
    failing = sum(c[k] for k in FAILING)
    print(
        f"== {fz.cases} cases in {el / 60:.1f} min ({fz.cases / max(el, 1e-9):.0f}/s): "
        + ", ".join(f"{k} {c[k]}" for k in CLASSES)
        + f"; failing {failing}"
    )
    sys.exit(1 if failing else 0)


if __name__ == "__main__":
    main()
