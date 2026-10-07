"""Generate the M3 fancy-upsampling corpus, corpus/synthetic-m3/.

Usage: python3 -I tools/gen_m3.py

Decodes corpus/primary/testorig.jpg (227x149) with the oracle's djpeg, crops
it at offset x=100, y=60 (the top-left corner is flat, where fancy and plain
upsampling coincide; here they differ), and compresses each crop with the
oracle's cjpeg `-dct int -optimize -sample S` (-optimize keeps the files
around 300 bytes, small enough to embed as unit-test fixtures). Names are
`m<w>x<h>-s<S>.jpg`, with the commas of S written as `-`, and `full-s<S>.jpg`
for the whole image.

- 14 crops, 3x3 to 33x34, each with 2x2, 2x1, 1x2, 2x2,2x1,1x1,
  2x2,1x2,1x1 and 1x4,1x2,1x1. Widths 3 to 6 sit on libjpeg's rule that
  h2v1 and h2v2 are fancy only when the downsampled width exceeds 2; even
  heights such as 20 and 18 leave a partial last MCU row, whose bottom
  context is the last real row; 6x20 and 16x18 have two MCU rows; the lists
  put h1v2 and h2v1 beside h2v2, and 1x4,1x2,1x1 gives h1v2 a two-row group.
- The full image with eight per-component lists.
- m16x18-s2x2-r1.jpg: restart markers every MCU row across the context.

Every file is checked to decode with both `djpeg -dct int -pnm` and
`djpeg -dct int -nosmooth -pnm`, exit status 0 and nothing on stderr. cjpeg
is deterministic, so two runs write identical files. Only
corpus/synthetic-m3/*.jpg is deleted and rewritten.
"""

import glob
import os
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "oracle", "libjpeg-turbo-3.2.0", "bin")
OUT = os.path.join(ROOT, "corpus", "synthetic-m3")
TMP = os.path.join(ROOT, "..", "tmp", "gen-m3")
X0, Y0 = 100, 60

CROPS = [(3, 3), (4, 4), (5, 5), (6, 6), (6, 4), (4, 6), (5, 6), (6, 5), (6, 20), (8, 20), (16, 18), (15, 17), (20, 10), (33, 34)]
CROP_SAMPLINGS = ["2x2", "2x1", "1x2", "2x2,2x1,1x1", "2x2,1x2,1x1", "1x4,1x2,1x1"]
FULL_SAMPLINGS = ["2x2,2x1,1x1", "2x2,1x2,1x1", "1x4,1x2,1x1", "4x1,2x1,1x1", "1x4,1x2,1x2", "4x1,2x1,2x1", "2x2,1x1,2x2", "1x2,1x1,1x2"]


def source():
    ppm = subprocess.run([os.path.join(BIN, "djpeg"), "-pnm", os.path.join(ROOT, "corpus", "primary", "testorig.jpg")],
                         capture_output=True, check=True).stdout
    magic, w, h, maxval, pixels = ppm.split(maxsplit=4)
    assert magic == b"P6" and maxval == b"255"
    return int(w), int(h), pixels


def crop(src, x0, y0, cw, ch):
    w, h, pixels = src
    path = os.path.join(TMP, f"c{cw}x{ch}+{x0}+{y0}.ppm")
    rows = b"".join(pixels[((y0 + y) * w + x0) * 3 : ((y0 + y) * w + x0 + cw) * 3] for y in range(ch))
    with open(path, "wb") as f:
        f.write(b"P6\n%d %d\n255\n" % (cw, ch) + rows)
    return path


def make(name, ppm, args):
    dst = os.path.join(OUT, name + ".jpg")
    subprocess.run([os.path.join(BIN, "cjpeg"), "-dct", "int", "-optimize", *args, "-outfile", dst, ppm], check=True)
    for flags in ([], ["-nosmooth"]):
        r = subprocess.run([os.path.join(BIN, "djpeg"), "-dct", "int", *flags, "-pnm", dst], capture_output=True)
        assert r.returncode == 0 and not r.stderr, (name, flags, r.returncode, r.stderr)


def tag(s):
    return s.replace(",", "-")


def main():
    os.makedirs(TMP, exist_ok=True)
    os.makedirs(OUT, exist_ok=True)
    for f in glob.glob(os.path.join(OUT, "*.jpg")):
        os.remove(f)
    src = source()
    for cw, ch in CROPS:
        ppm = crop(src, X0, Y0, cw, ch)
        for s in CROP_SAMPLINGS:
            make(f"m{cw}x{ch}-s{tag(s)}", ppm, ["-sample", s])
    full = crop(src, 0, 0, src[0], src[1])
    for s in FULL_SAMPLINGS:
        make(f"full-s{tag(s)}", full, ["-sample", s])
    make("m16x18-s2x2-r1", crop(src, X0, Y0, 16, 18), ["-sample", "2x2", "-restart", "1"])
    print(f"gen_m3: {len(glob.glob(os.path.join(OUT, '*.jpg')))} files in corpus/synthetic-m3")


if __name__ == "__main__":
    main()
