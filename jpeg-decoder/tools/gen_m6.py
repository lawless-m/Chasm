"""Generate the M6 12-bit corpus, corpus/synthetic-m6/.

Usage: python3 -I tools/gen_m6.py

12-bit samples through every coding the decoder handles: extended
sequential Huffman (SOF1), progressive (SOF2), arithmetic sequential and
progressive (SOF9, SOF10). The source is djpeg's 12-bit decode of
corpus/primary/testorig12.jpg (227x149, maxval 4095), cropped at offset
(100, 60) where a size asks for it, and two flat maxval-4095 images; every
cjpeg call carries `-precision 12 -dct int`. Files are named
`<size>-<tag>-<variant>.jpg`, size `full`, `c<w>x<h>`, `flat64x48` or
`flatgrey48x40`, tag `s<sampling>` or `grey`:

- `seq`: every sampling cjpeg accepts on the full image, and crops from 1x1
  to 64x48 in 2x2, 2x1, 1x2 and grey, so partial MCUs and the fancy
  upsampler's downsampled-width-2 rule are hit at 12 bits
- `seq-r*`: restart intervals in MCU rows and in MCUs
- `seq-q1`, `seq-q3`, `seq-q100`: 16-bit quantisation tables and large
  coefficients (DC categories near 15) and range clamping
- `prog`, `prog-r*`: libjpeg's standard progressive script, with restarts
- the scan scripts of tools/gen_m4.py (DC only, coarse, partial, deep and
  separate DC scans: block smoothing at 12 bits; multi-scan sequential)
- `arith*`: arithmetic coding, sequential and progressive, with restarts,
  16-bit tables and scan scripts
- flat images: long zero runs
- `tw-<stem>-<variant>`: jpegtran twins of the eight 12-bit originals
  (testorig12 and corpus/synthetic/p12-*), arithmetic, progressive, restart
  every MCU row, and progressive with a restart every four MCUs; a twin keeps
  its original's coefficients, so it must decode to the same pixels

Every file is checked to decode with both `djpeg -dct int -pnm` and
`djpeg -dct int -nosmooth -pnm`, exit status 0 and nothing on stderr, and
each twin to give exactly its original's output in both. cjpeg and
jpegtran are deterministic, so two runs write identical files. Only
corpus/synthetic-m6/*.jpg is deleted and rewritten.
"""

import glob
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "oracle", "libjpeg-turbo-3.2.0", "bin")
OUT = os.path.join(ROOT, "corpus", "synthetic-m6")
TMP = os.path.join(ROOT, "..", "tmp", "gen-m6")
X0, Y0 = 100, 60
ORIGINALS = ["primary/testorig12.jpg"] + [f"synthetic/p12-{s}.jpg" for s in
                                          ("arith-420", "arith-prog-420", "grey", "prog-420", "seq-420", "seq-422-rst2", "seq-444")]

sys.path.insert(0, os.path.join(ROOT, "tools"))
import gen_m4  # noqa: E402

gen_m4.TMP = TMP


def fail(msg):
    raise SystemExit(f"gen_m6: {msg}")


def djpeg(path, flags):
    r = subprocess.run([os.path.join(BIN, "djpeg"), "-dct", "int", *flags, "-pnm", path], capture_output=True)
    if r.returncode or r.stderr:
        fail(f"{path}: djpeg {flags} exit {r.returncode} {r.stderr!r}")
    return r.stdout


def source():
    ppm = subprocess.run([os.path.join(BIN, "djpeg"), "-dct", "int", "-pnm", os.path.join(ROOT, "corpus", "primary", "testorig12.jpg")],
                         capture_output=True, check=True).stdout
    magic, w, h, maxval, pixels = ppm.split(maxsplit=4)
    assert magic == b"P6" and maxval == b"4095" and len(pixels) == int(w) * int(h) * 6
    return int(w), int(h), pixels


def crop(src, size):
    w, h, pixels = src
    cw, ch = size
    x0, y0 = (0, 0) if size == (w, h) else (X0, Y0)
    path = os.path.join(TMP, f"c{cw}x{ch}.ppm")
    rows = b"".join(pixels[((y0 + y) * w + x0) * 6 : ((y0 + y) * w + x0 + cw) * 6] for y in range(ch))
    with open(path, "wb") as f:
        f.write(b"P6\n%d %d\n4095\n" % (cw, ch) + rows)
    return path


def flat(name, magic, w, h, sample):
    path = os.path.join(TMP, name)
    with open(path, "wb") as f:
        f.write(b"%s\n%d %d\n4095\n" % (magic, w, h) + b"".join(s.to_bytes(2, "big") for s in sample) * (w * h))
    return path


def make(name, ppm, args):
    dst = os.path.join(OUT, name + ".jpg")
    r = subprocess.run([os.path.join(BIN, "cjpeg"), "-precision", "12", "-dct", "int", *args, "-outfile", dst, ppm], capture_output=True)
    if r.returncode:
        fail(f"{name}: cjpeg exit {r.returncode} {r.stderr!r}")
    djpeg(dst, [])
    djpeg(dst, ["-nosmooth"])


def twin(src, name, args):
    dst = os.path.join(OUT, name + ".jpg")
    r = subprocess.run([os.path.join(BIN, "jpegtran"), *args, "-outfile", dst, src], capture_output=True)
    if r.returncode or r.stderr:
        fail(f"{name}: jpegtran exit {r.returncode} {r.stderr!r}")
    for flags in ([], ["-nosmooth"]):
        if djpeg(dst, flags) != djpeg(src, flags):
            fail(f"{name}: differs from {src} with djpeg {flags}")


def samp(s):
    return ["-grayscale"] if s == "grey" else ["-sample", s]


def tag(s):
    return "grey" if s == "grey" else "s" + s.replace(",", "-")


def main():
    os.makedirs(TMP, exist_ok=True)
    os.makedirs(OUT, exist_ok=True)
    for f in glob.glob(os.path.join(OUT, "*.jpg")):
        os.remove(f)
    src = source()
    full = crop(src, (src[0], src[1]))
    for s in ("2x2", "1x1", "2x1", "1x2", "4x1", "4x2", "3x1", "1x3", "2x2,1x2,1x1", "grey"):
        make(f"full-{tag(s)}-seq", full, samp(s))
    for size in [(1, 1), (2, 1), (1, 2), (3, 3), (5, 5), (7, 5), (8, 8), (9, 9), (15, 17), (16, 16), (17, 15), (33, 31), (64, 48)]:
        ppm = crop(src, size)
        for s in ("2x2", "2x1", "1x2", "grey"):
            make(f"c{size[0]}x{size[1]}-{tag(s)}-seq", ppm, samp(s))
    for s, r in (("2x2", "1"), ("2x2", "2B"), ("2x2", "3"), ("grey", "1B")):
        make(f"full-{tag(s)}-seq-r{r}", full, samp(s) + ["-restart", r])
    for s, q in (("2x2", "1"), ("2x2", "3"), ("2x2", "100"), ("grey", "1"), ("grey", "100"), ("1x1", "100")):
        make(f"full-{tag(s)}-seq-q{q}", full, samp(s) + ["-quality", q])
    for s in ("2x2", "1x1", "2x1", "grey"):
        make(f"full-{tag(s)}-prog", full, samp(s) + ["-progressive"])
    for size in [(1, 1), (7, 5), (17, 15), (33, 31)]:
        ppm = crop(src, size)
        for s in ("2x2", "grey"):
            make(f"c{size[0]}x{size[1]}-{tag(s)}-prog", ppm, samp(s) + ["-progressive"])
    for s, r in (("2x2", "1"), ("2x2", "4B"), ("grey", "2B")):
        make(f"full-{tag(s)}-prog-r{r}", full, samp(s) + ["-progressive", "-restart", r])
    c17 = crop(src, (17, 15))
    for size, ppm in (("full", full), ("c17x15", c17)):
        for v in ("dc", "dcal", "coarse", "spectral", "partial", "bands", "deep", "dcsep", "dcsep-only"):
            make(f"{size}-s2x2-{v}", ppm, ["-sample", "2x2", "-scans", gen_m4.script(v)])
    for v in ("dc", "dcal", "coarse", "spectral", "partial"):
        make(f"full-grey-{v}", full, ["-grayscale", "-scans", gen_m4.script("g" + v)])
    for v in ("seq3", "seq21", "seq-rev"):
        make(f"full-s2x2-{v}", full, ["-sample", "2x2", "-scans", gen_m4.script(v)])
    make("full-s2x2-seq3-q3", full, ["-sample", "2x2", "-scans", gen_m4.script("seq3"), "-quality", "3"])
    for s in ("2x2", "1x1", "grey"):
        make(f"full-{tag(s)}-arith", full, samp(s) + ["-arithmetic"])
    for s in ("2x2", "grey"):
        make(f"full-{tag(s)}-arith-prog", full, samp(s) + ["-arithmetic", "-progressive"])
    make("full-s2x2-arith-r1", full, ["-sample", "2x2", "-arithmetic", "-restart", "1"])
    make("full-s2x2-arith-q1", full, ["-sample", "2x2", "-arithmetic", "-quality", "1"])
    for v in ("dc", "coarse", "deep"):
        make(f"full-s2x2-arith-{v}", full, ["-sample", "2x2", "-arithmetic", "-scans", gen_m4.script(v)])
    make("c17x15-s2x2-arith-prog", c17, ["-sample", "2x2", "-arithmetic", "-progressive"])
    make("c17x15-grey-arith", c17, ["-grayscale", "-arithmetic"])
    fl = flat("flat64x48.ppm", b"P6", 64, 48, [3000, 1500, 4000])
    make("flat64x48-s2x2-prog", fl, ["-sample", "2x2", "-progressive"])
    make("flat64x48-s2x2-prog-r1", fl, ["-sample", "2x2", "-progressive", "-restart", "1"])
    fg = flat("flatgrey48x40.pgm", b"P5", 48, 40, [2560])
    make("flatgrey48x40-prog", fg, ["-grayscale", "-progressive"])
    make("flatgrey48x40-prog-r2B", fg, ["-grayscale", "-progressive", "-restart", "2B"])
    for o in ORIGINALS:
        path = os.path.join(ROOT, "corpus", o)
        stem = os.path.basename(o)[:-4]
        for v, args in (("arith", ["-arithmetic"]), ("prog", ["-progressive"]), ("r1", ["-restart", "1"]), ("pr4B", ["-progressive", "-restart", "4B"])):
            twin(path, f"tw-{stem}-{v}", args)
    print(f"gen_m6: {len(glob.glob(os.path.join(OUT, '*.jpg')))} files in corpus/synthetic-m6")


if __name__ == "__main__":
    main()
