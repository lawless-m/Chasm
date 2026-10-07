"""Generate the M2 baseline corpus, corpus/synthetic-m2/.

Usage: python3 -I tools/gen_m2.py

Decodes corpus/primary/testorig.jpg (227x149) with the oracle's djpeg,
crops it where a variant asks for a smaller size, and compresses it again
with the oracle's cjpeg (`-dct int`, quality 75 unless a variant says
otherwise). Files are named `<size>-<variant>.jpg`, size `full` or
`c<w>x<h>`:

- every sampling factor cjpeg accepts on the full image: 1x1, 2x1, 1x2, 2x2,
  4x1, 1x4, 4x2, 2x4, 3x1, 1x3, and the lists 2x1,1x2,1x1 and 4x1,1x1,2x1
  (cjpeg refuses 4x4 and 3x3 for an interleaved scan)
- for each crop 1x1 2x1 1x2 7x5 8x8 9x9 15x17 16x16 17x15 33x31 64x48:
  colour 2x2, 1x1 and 2x1, and greyscale
- restart intervals: in MCU rows (`r1`, `r2`) and in MCUs (`r1B`, `r3B`,
  `r7B`, `r5B`), with several samplings and in greyscale
- greyscale with 2x2 sampling (one component on its own block grid) for the
  full image, c17x15 and c16x24
- `-rgb`; `-quality 100`; `-quality 1` and `-quality 3` (16-bit
  quantisation tables, SOF1); `-optimize` in colour and greyscale

cjpeg is deterministic, so two runs write identical files. Every file is
checked to decode with `djpeg -dct int -nosmooth -pnm` with exit status 0
and nothing on stderr, so each one is a byte-exact expectation for M2.
Only corpus/synthetic-m2/*.jpg is deleted and rewritten.
"""

import glob
import os
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "oracle", "libjpeg-turbo-3.2.0", "bin")
OUT = os.path.join(ROOT, "corpus", "synthetic-m2")
TMP = os.path.join(ROOT, "..", "tmp", "gen-m2")

SAMPLINGS = ["1x1", "2x1", "1x2", "2x2", "4x1", "1x4", "4x2", "2x4", "3x1", "1x3", "2x1,1x2,1x1", "4x1,1x1,2x1"]
CROPS = [(1, 1), (2, 1), (1, 2), (7, 5), (8, 8), (9, 9), (15, 17), (16, 16), (17, 15), (33, 31), (64, 48)]


def source():
    ppm = subprocess.run([os.path.join(BIN, "djpeg"), "-pnm", os.path.join(ROOT, "corpus", "primary", "testorig.jpg")],
                         capture_output=True, check=True).stdout
    magic, w, h, maxval, pixels = ppm.split(maxsplit=4)
    assert magic == b"P6" and maxval == b"255"
    return int(w), int(h), pixels


def crop(src, size):
    w, h, pixels = src
    cw, ch = size
    path = os.path.join(TMP, f"c{cw}x{ch}.ppm")
    rows = b"".join(pixels[(y * w) * 3 : (y * w + cw) * 3] for y in range(ch))
    with open(path, "wb") as f:
        f.write(b"P6\n%d %d\n255\n" % (cw, ch) + rows)
    return path


def make(name, ppm, args):
    dst = os.path.join(OUT, name + ".jpg")
    subprocess.run([os.path.join(BIN, "cjpeg"), "-dct", "int", *args, "-outfile", dst, ppm], check=True)
    r = subprocess.run([os.path.join(BIN, "djpeg"), "-dct", "int", "-nosmooth", "-pnm", dst], capture_output=True)
    assert r.returncode == 0 and not r.stderr, (name, r.returncode, r.stderr)


def main():
    os.makedirs(TMP, exist_ok=True)
    os.makedirs(OUT, exist_ok=True)
    for f in glob.glob(os.path.join(OUT, "*.jpg")):
        os.remove(f)
    src = source()
    full = crop(src, (src[0], src[1]))
    for s in SAMPLINGS:
        make("full-s" + s.replace(",", "-"), full, ["-sample", s])
    for size in CROPS:
        ppm = crop(src, size)
        tag = f"c{size[0]}x{size[1]}"
        for s in ("2x2", "1x1", "2x1"):
            make(f"{tag}-s{s}", ppm, ["-sample", s])
        make(f"{tag}-grey", ppm, ["-grayscale"])
    for name, args in [
        ("full-s2x2-r1", ["-sample", "2x2", "-restart", "1"]),
        ("full-s2x2-r1B", ["-sample", "2x2", "-restart", "1B"]),
        ("full-s2x2-r3B", ["-sample", "2x2", "-restart", "3B"]),
        ("full-s2x2-r7B", ["-sample", "2x2", "-restart", "7B"]),
        ("full-s4x1-r2", ["-sample", "4x1", "-restart", "2"]),
        ("full-s1x1-r1B", ["-sample", "1x1", "-restart", "1B"]),
        ("full-grey-r5B", ["-grayscale", "-restart", "5B"]),
        ("full-grey-s2x2-r1", ["-grayscale", "-sample", "2x2", "-restart", "1"]),
        ("full-grey-s2x2", ["-grayscale", "-sample", "2x2"]),
        ("full-rgb", ["-rgb"]),
        ("full-s2x2-q100", ["-sample", "2x2", "-quality", "100"]),
        ("full-s2x2-q1", ["-sample", "2x2", "-quality", "1"]),
        ("full-s1x1-q3", ["-sample", "1x1", "-quality", "3"]),
        ("full-s2x2-opt", ["-sample", "2x2", "-optimize"]),
        ("full-grey-opt", ["-grayscale", "-optimize"]),
    ]:
        make(name, full, args)
    for size in [(17, 15), (16, 24)]:
        make(f"c{size[0]}x{size[1]}-grey-s2x2", crop(src, size), ["-grayscale", "-sample", "2x2"])
    print(f"gen_m2: {len(glob.glob(os.path.join(OUT, '*.jpg')))} files in corpus/synthetic-m2")


if __name__ == "__main__":
    main()
