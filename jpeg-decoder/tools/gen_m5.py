"""Generate the M5 arithmetic corpus, corpus/synthetic-m5/.

Usage: python3 -I tools/gen_m5.py

Twins. `jpegtran -arithmetic` transcodes a Huffman JPEG to arithmetic coding
without touching its quantised coefficients, so a twin decodes to exactly the
pixels of its original. Every baseline file the decoder already decodes
byte-exact (tools/m2-decoded.txt, corpus/synthetic-m2, corpus/synthetic-m3)
gets four twins: `seq` (SOF9), `prog` (SOF10, libjpeg's standard ten
scans: DC first and refine, AC first and refine), `r1` (SOF9 with a restart
every MCU row) and `pr4B` (SOF10 with a restart every four MCUs, so EOB and
predictor state, statistics and the coder are reset inside every kind of
scan). A twin is named `tw-<dir>-<stem>-<variant>.jpg`, dir being the
original's directory (primary, imagers, mozilla, pillow, synthetic-m2,
synthetic-m3). Identical output to the original proves the entropy decoder,
since everything after it was proved in M2 to M4.

cjpeg files. The M4 set is not twinned: a sequential twin of an incomplete
progressive original loses the original's block smoothing. Instead the 83
invocations of tools/gen_m4.py are rerun (imported from it, so the two sets
cannot drift) with `-arithmetic` added, named `a-<gen_m4 name>.jpg`; each
must decode to the same pixels as its synthetic-m4 counterpart. These
cover what jpegtran cannot change: scan scripts (DC only, deep successive
approximation, separate DC scans, smoothing cases), greyscale, every
sampling, flat images with long zero runs, and 16-bit tables (SOF9 at
`-quality 3`).

Checks: jpegtran exits 0 with nothing on stderr; every file decodes with
both `djpeg -dct int -pnm` and `djpeg -dct int -nosmooth -pnm`, exit 0 and
nothing on stderr, to exactly what the original (or the synthetic-m4
counterpart) gives. Any failure stops the script. cjpeg and jpegtran are
deterministic, so two runs write identical files. Only
corpus/synthetic-m5/*.jpg is deleted and rewritten.
"""

import glob
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "oracle", "libjpeg-turbo-3.2.0", "bin")
OUT = os.path.join(ROOT, "corpus", "synthetic-m5")
TMP = os.path.join(ROOT, "..", "tmp", "gen-m5")
VARIANTS = {"seq": [], "prog": ["-progressive"], "r1": ["-restart", "1"], "pr4B": ["-progressive", "-restart", "4B"]}

sys.path.insert(0, os.path.join(ROOT, "tools"))
import gen_m4  # noqa: E402


def djpeg(path, flags):
    r = subprocess.run([os.path.join(BIN, "djpeg"), "-dct", "int", *flags, "-pnm", path], capture_output=True)
    if r.returncode or r.stderr:
        raise SystemExit(f"gen_m5: {path}: djpeg {flags} exit {r.returncode} {r.stderr!r}")
    return r.stdout


def same(made, ref):
    for flags in ([], ["-nosmooth"]):
        if djpeg(made, flags) != djpeg(ref, flags):
            raise SystemExit(f"gen_m5: {made}: differs from {ref} with djpeg {flags}")


def twin(job):
    src, name, args = job
    dst = os.path.join(OUT, name)
    r = subprocess.run([os.path.join(BIN, "jpegtran"), "-arithmetic", *args, "-outfile", dst, src], capture_output=True)
    if r.returncode or r.stderr:
        raise SystemExit(f"gen_m5: {name}: jpegtran exit {r.returncode} {r.stderr!r}")
    same(dst, src)


def originals():
    with open(os.path.join(ROOT, "tools", "m2-decoded.txt")) as f:
        paths = [os.path.join(ROOT, line.strip()) for line in f if line.strip()]
    for d in ("synthetic-m2", "synthetic-m3"):
        paths += glob.glob(os.path.join(ROOT, "corpus", d, "*.jpg"))
    return sorted(paths)


def main():
    os.makedirs(TMP, exist_ok=True)
    os.makedirs(OUT, exist_ok=True)
    for f in glob.glob(os.path.join(OUT, "*.jpg")):
        os.remove(f)
    jobs = []
    for src in originals():
        d = os.path.basename(os.path.dirname(src))
        stem = os.path.basename(src)[:-4]
        for v, args in VARIANTS.items():
            jobs.append((src, f"tw-{d}-{stem}-{v}.jpg", args))
    names = [j[1] for j in jobs]
    assert len(names) == len(set(names)) == 952, len(set(names))
    with ThreadPoolExecutor(os.cpu_count()) as ex:
        list(ex.map(twin, jobs))

    def make(name, ppm, args):
        dst = os.path.join(OUT, f"a-{name}.jpg")
        subprocess.run([os.path.join(BIN, "cjpeg"), "-dct", "int", "-arithmetic", *args, "-outfile", dst, ppm], check=True,
                       stderr=subprocess.DEVNULL)
        same(dst, os.path.join(ROOT, "corpus", "synthetic-m4", name + ".jpg"))

    gen_m4.make, gen_m4.OUT, gen_m4.TMP = make, OUT, TMP
    real_glob = gen_m4.glob.glob
    gen_m4.glob.glob = lambda pat: [] if pat == os.path.join(OUT, "*.jpg") else real_glob(pat)
    try:
        with open(os.devnull, "w") as quiet:
            saved, sys.stdout = sys.stdout, quiet
            gen_m4.main()
    finally:
        sys.stdout = saved
        gen_m4.glob.glob = real_glob
    print(f"gen_m5: {len(glob.glob(os.path.join(OUT, '*.jpg')))} files in corpus/synthetic-m5")


if __name__ == "__main__":
    main()
