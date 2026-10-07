"""Generate the M4 progressive and multi-scan corpus, corpus/synthetic-m4/.

Usage: python3 -I tools/gen_m4.py

Decodes corpus/primary/testorig.jpg (227x149) with the oracle's djpeg, crops
it at offset (100, 60) where a size asks for it (as gen_m3.py), or makes a
flat image, and compresses it with the oracle's cjpeg (`-dct int`). Names
are `<size>-<sampling>-<variant>.jpg`: size `full`, `c<w>x<h>`,
`flat64x48` or `flatgrey48x40`; sampling `s<S>` (commas written `-`) or
`grey`. The groups:

- `prog`: libjpeg's standard progressive script on several samplings and
  on crops from 1x1 to 64x48, so interleaved DC scans run over padded
  grids and AC scans over each component's unpadded grid.
- `prog-r*`: the same with restart intervals (in MCU rows or in MCUs), so
  EOB runs and predictors reset at restarts.
- scan scripts: DC only (`dc`), DC in two steps (`dcal`), AC left at Al 2
  and 1 (`coarse`: smoothing with the Al clamps), spectral bands without
  refinement (`spectral`, `bands`), coefficients never sent (`partial`:
  smoothing without clamps), successive approximation to depth 3 (`deep`),
  separate DC scans per component (`dcsep`, `dcsep-only`: DC
  interpolation), on colour and greyscale.
- flat images: long EOB runs (EOBn with extra bits), with and without
  restarts.
- multi-scan sequential: one scan per component or two components then one
  (`seq3`, `seq21`), reversed component order (`seq-rev`), with restarts,
  16-bit tables (SOF1, `-quality 3`) and `-optimize`.

Every file is checked to decode with both `djpeg -dct int -pnm` and
`djpeg -dct int -nosmooth -pnm`, exit status 0 and nothing on stderr.
cjpeg is deterministic, so two runs write identical files. Only
corpus/synthetic-m4/*.jpg is deleted and rewritten.
"""

import glob
import os
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "oracle", "libjpeg-turbo-3.2.0", "bin")
OUT = os.path.join(ROOT, "corpus", "synthetic-m4")
TMP = os.path.join(ROOT, "..", "tmp", "gen-m4")
X0, Y0 = 100, 60

SCRIPTS = {
    "dc": "0,1,2: 0-0, 0, 0;",
    "dcal": "0,1,2: 0-0, 0, 1; 0,1,2: 0-0, 1, 0;",
    "coarse": "0,1,2: 0-0,0,1; 0: 1-5,0,2; 1: 1-63,0,1; 2: 1-63,0,1; 0: 6-63,0,2; 0,1,2: 0-0,1,0;",
    "spectral": "0,1,2: 0-0,0,0; 0: 1-5,0,0; 0: 6-63,0,0; 1: 1-63,0,0; 2: 1-63,0,0;",
    "partial": "0,1,2: 0-0,0,0; 0: 1-9,0,0; 1: 1-2,0,0; 2: 1-2,0,0;",
    "bands": "0,1,2: 0-0,0,0; 0: 1-1,0,0; 0: 2-2,0,0; 0: 3-5,0,0; 0: 6-9,0,0; 0: 10-20,0,0; 0: 21-63,0,0; 1: 1-63,0,0; 2: 1-63,0,0;",
    "deep": "0,1,2: 0-0,0,2; 0: 1-5,0,3; 0: 6-63,0,3; 0: 1-63,3,2; 0: 1-63,2,1; 0,1,2: 0-0,2,1; 0,1,2: 0-0,1,0; 0: 1-63,1,0; 1: 1-63,0,0; 2: 1-63,0,0;",
    "dcsep": "0: 0-0,0,1; 1: 0-0,0,1; 2: 0-0,0,1; 0: 1-63,0,0; 1: 1-63,0,0; 2: 1-63,0,0; 0: 0-0,1,0; 1: 0-0,1,0; 2: 0-0,1,0;",
    "dcsep-only": "0: 0-0,0,0; 1: 0-0,0,0; 2: 0-0,0,0;",
    "seq3": "0: 0-63,0,0; 1: 0-63,0,0; 2: 0-63,0,0;",
    "seq21": "0,1: 0-63,0,0; 2: 0-63,0,0;",
    "seq-rev": "2: 0-63,0,0; 1: 0-63,0,0; 0: 0-63,0,0;",
    "gdc": "0: 0-0, 0, 0;",
    "gdcal": "0: 0-0,0,1; 0: 0-0,1,0;",
    "gcoarse": "0: 0-0,0,1; 0: 1-5,0,2; 0: 6-63,0,2; 0: 0-0,1,0;",
    "gspectral": "0: 0-0,0,0; 0: 1-5,0,0; 0: 6-63,0,0;",
    "gpartial": "0: 0-0,0,0; 0: 1-9,0,0;",
}


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


def flat(name, magic, w, h, pixel):
    path = os.path.join(TMP, name)
    with open(path, "wb") as f:
        f.write(b"%s\n%d %d\n255\n" % (magic, w, h) + bytes(pixel) * (w * h))
    return path


def script(name):
    path = os.path.join(TMP, name + ".scans")
    with open(path, "w") as f:
        f.write(SCRIPTS[name].replace("; ", ";\n") + "\n")
    return path


def sampling(s):
    return ["-grayscale"] if s == "grey" else ["-sample", s]


def tag(s):
    return "grey" if s == "grey" else "s" + s.replace(",", "-")


def make(name, ppm, args):
    dst = os.path.join(OUT, name + ".jpg")
    subprocess.run([os.path.join(BIN, "cjpeg"), "-dct", "int", *args, "-outfile", dst, ppm], check=True,
                   stderr=subprocess.DEVNULL)
    for flags in ([], ["-nosmooth"]):
        r = subprocess.run([os.path.join(BIN, "djpeg"), "-dct", "int", *flags, "-pnm", dst], capture_output=True)
        assert r.returncode == 0 and not r.stderr, (name, flags, r.returncode, r.stderr)


def main():
    os.makedirs(TMP, exist_ok=True)
    os.makedirs(OUT, exist_ok=True)
    for f in glob.glob(os.path.join(OUT, "*.jpg")):
        os.remove(f)
    src = source()
    full = crop(src, 0, 0, src[0], src[1])
    for s in ("2x2", "1x1", "2x1", "1x2", "2x2,1x2,1x1", "4x1", "grey"):
        make(f"full-{tag(s)}-prog", full, sampling(s) + ["-progressive"])
    for cw, ch in [(1, 1), (2, 1), (1, 2), (7, 5), (8, 8), (9, 9), (15, 17), (16, 16), (17, 15), (33, 31), (64, 48)]:
        ppm = crop(src, X0, Y0, cw, ch)
        for s in ("2x2", "grey"):
            make(f"c{cw}x{ch}-{tag(s)}-prog", ppm, sampling(s) + ["-progressive"])
    c17 = crop(src, X0, Y0, 17, 15)
    for size, ppm, s, r in [("full", full, "2x2", "1"), ("full", full, "2x2", "1B"), ("full", full, "2x2", "3B"),
                            ("full", full, "2x2", "2"), ("full", full, "grey", "1B"), ("full", full, "grey", "5B"),
                            ("c17x15", c17, "2x2", "1")]:
        make(f"{size}-{tag(s)}-prog-r{r}", ppm, sampling(s) + ["-progressive", "-restart", r])
    colour = ["dc", "dcal", "coarse", "spectral", "partial", "bands", "deep", "dcsep", "dcsep-only"]
    for size, ppm in [("full", full), ("c17x15", c17)]:
        for v in colour:
            make(f"{size}-s2x2-{v}", ppm, ["-sample", "2x2", "-scans", script(v)])
    for size, ppm in [("full", full), ("c17x15", c17)]:
        for v in ("dc", "dcal", "coarse", "spectral", "partial"):
            make(f"{size}-grey-{v}", ppm, ["-grayscale", "-scans", script("g" + v)])
    for v in ("dc", "coarse", "deep"):
        make(f"full-s1x1-{v}", full, ["-sample", "1x1", "-scans", script(v)])
    fl = flat("flat64x48.ppm", b"P6", 64, 48, [90, 140, 200])
    make("flat64x48-s2x2-prog", fl, ["-sample", "2x2", "-progressive"])
    make("flat64x48-s2x2-prog-r1", fl, ["-sample", "2x2", "-progressive", "-restart", "1"])
    make("flat64x48-s2x2-coarse", fl, ["-sample", "2x2", "-scans", script("coarse")])
    make("flat64x48-s2x2-dc", fl, ["-sample", "2x2", "-scans", script("dc")])
    fg = flat("flatgrey48x40.pgm", b"P5", 48, 40, [77])
    make("flatgrey48x40-prog", fg, ["-grayscale", "-progressive"])
    make("flatgrey48x40-prog-r2B", fg, ["-grayscale", "-progressive", "-restart", "2B"])
    for v in ("seq3", "seq21", "seq-rev"):
        make(f"full-s2x2-{v}", full, ["-sample", "2x2", "-scans", script(v)])
    make("full-s1x1-seq3", full, ["-sample", "1x1", "-scans", script("seq3")])
    make("full-s2x1-seq3", full, ["-sample", "2x1", "-scans", script("seq3")])
    make("c17x15-s2x2-seq3", c17, ["-sample", "2x2", "-scans", script("seq3")])
    make("c9x9-s2x2-seq-rev", crop(src, X0, Y0, 9, 9), ["-sample", "2x2", "-scans", script("seq-rev")])
    make("full-s2x2-seq3-r2", full, ["-sample", "2x2", "-scans", script("seq3"), "-restart", "2"])
    make("full-s2x2-seq3-q3", full, ["-sample", "2x2", "-scans", script("seq3"), "-quality", "3"])
    make("full-s2x2-seq3-opt", full, ["-sample", "2x2", "-scans", script("seq3"), "-optimize"])
    print(f"gen_m4: {len(glob.glob(os.path.join(OUT, '*.jpg')))} files in corpus/synthetic-m4")


if __name__ == "__main__":
    main()
