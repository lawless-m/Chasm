"""Decode every file of the decoder's corpus with the oracle's djpeg, keeping
the decodes the encoder takes as `encode` inputs.

Usage: python3 -I tools/decode_corpus.py [--jobs N] [--timeout SECONDS] [--keep-p12 DIR] OUTDIR

Walks ../jpeg-decoder/corpus recursively in sorted order (names ending in
.md or .txt skipped) and runs `djpeg -dct int -pnm -outfile <tmp> <file>` on
each. Classes:

- refused: djpeg exited non-zero, timed out or wrote nothing
- p12:     maxval 4095, a 12-bit decode; deleted unless --keep-p12 DIR,
           which moves it to DIR, named as a kept decode is
- other:   any other maxval but 255
- large:   width x height over 80000000 (MAX_PIXELS in
           ../jpeg-decoder/jpeg/limits.wack: ppmread refuses such an image
           and the decoder never produces one)
- kept:    everything else; moved to OUTDIR/<path under corpus, '/' as '_'>
           with the extension .pgm (P5) or .ppm (P6)

Every other decode is deleted. OUTDIR/manifest.txt has one line a
corpus file, `<class> <path> [<w>x<h> <P5|P6> <maxval>]`, in corpus order,
and a last line `== decoded D kept K p12 T other O large L refused R`
(decoded = kept + p12 + other + large), also printed to stdout. The kept
files are the E2 gate's corpus inputs and the --keep-p12 files the E3
gate's; the classes and the manifest are the same with or without
--keep-p12, and rerunning gives the same files and manifest.
"""

import argparse
import concurrent.futures
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CORPUS = os.path.join(ROOT, "..", "jpeg-decoder", "corpus")
DJPEG = os.path.join(ROOT, "..", "jpeg-decoder", "oracle", "libjpeg-turbo-3.2.0", "bin", "djpeg")
MAX_PIXELS = 80000000


def corpus_files():
    out = []
    for d, dirs, names in os.walk(CORPUS):
        dirs.sort()
        for n in sorted(names):
            if not n.endswith((".md", ".txt")):
                out.append(os.path.relpath(os.path.join(d, n), CORPUS))
    return out


def header(path):
    """(magic, width, height, maxval) of a PNM file."""
    with open(path, "rb") as f:
        head = f.read(64)
    fields = head.split(None, 4)
    return fields[0].decode(), int(fields[1]), int(fields[2]), int(fields[3])


def decode(rel, outdir, timeout, p12dir):
    flat = rel.replace("/", "_")
    tmp = os.path.join(outdir, "tmp-" + flat + ".pnm")
    try:
        r = subprocess.run([DJPEG, "-dct", "int", "-pnm", "-outfile", tmp, os.path.join(CORPUS, rel)],
                           capture_output=True, timeout=timeout)
        ok = r.returncode == 0 and os.path.isfile(tmp) and os.path.getsize(tmp) > 0
    except subprocess.TimeoutExpired:
        ok = False
    if not ok:
        if os.path.exists(tmp):
            os.remove(tmp)
        return "refused", ""
    magic, w, h, maxval = header(tmp)
    info = f"{w}x{h} {magic} {maxval}"
    if maxval == 4095:
        cls = "p12"
    elif maxval != 255:
        cls = "other"
    elif w * h > MAX_PIXELS:
        cls = "large"
    else:
        cls = "kept"
    name = os.path.splitext(flat)[0] + (".pgm" if magic == "P5" else ".ppm")
    if cls == "kept":
        os.replace(tmp, os.path.join(outdir, name))
    elif cls == "p12" and p12dir:
        os.replace(tmp, os.path.join(p12dir, name))
    else:
        os.remove(tmp)
    return cls, info


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--timeout", type=float, default=120)
    ap.add_argument("--keep-p12", metavar="DIR")
    ap.add_argument("outdir")
    a = ap.parse_args()
    if not os.path.isfile(DJPEG):
        sys.exit(f"decode_corpus: {DJPEG} is missing")
    os.makedirs(a.outdir, exist_ok=True)
    if a.keep_p12:
        os.makedirs(a.keep_p12, exist_ok=True)
    files = corpus_files()
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        results = list(pool.map(lambda f: decode(f, a.outdir, a.timeout, a.keep_p12), files))
    counts = {k: 0 for k in ("kept", "p12", "other", "large", "refused")}
    lines = []
    for rel, (cls, info) in zip(files, results):
        counts[cls] += 1
        lines.append(f"{cls} {rel}" + (f" {info}" if info else ""))
    decoded = counts["kept"] + counts["p12"] + counts["other"] + counts["large"]
    last = (f"== decoded {decoded} kept {counts['kept']} p12 {counts['p12']} other {counts['other']} "
            f"large {counts['large']} refused {counts['refused']}")
    with open(os.path.join(a.outdir, "manifest.txt"), "w") as f:
        f.write("\n".join(lines + [last]) + "\n")
    print(last)


if __name__ == "__main__":
    main()
