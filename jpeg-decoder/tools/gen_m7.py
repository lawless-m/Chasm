"""Generate the M7 4-component corpus, corpus/synthetic-m7/.

Usage: python3 -I tools/gen_m7.py

CMYK and YCCK through every coding the decoder handles (SOF0, SOF1, SOF2,
SOF9, SOF10), at 8 and 12 bits, and header variants that pin libjpeg's
colour-space decision (jdapimin.c default_decompress_parms). djpeg writes
CMYK and YCCK to PPM as RGB through cmyk.h's cmyk_to_rgb, with no Adobe
inversion, so these files test the colour stage too. Groups:

- `pil-*`: Pillow CMYK (Adobe transform 0, ids C M Y K, only the first
  component subsampled, K unsampled) of djpeg's decode of
  corpus/primary/testorig.jpg: 4:4:4, 4:2:2 and 4:2:0 at two qualities;
  crops from 1x1 to 64x48 (partial MCUs, the downsampled-width-2 rule);
  quality 1 and 100; Huffman optimisation; progressive; restarts in MCU
  rows and in MCUs
- `tj-*`: tjbench YCCK (Adobe transform 2, ids 1 2 3 4, K sampled like Y)
  over every subsampling it writes (444 422 440 420 411 441), crops,
  progressive, arithmetic, restarts, quality extremes
- `tj12-*`: the same at 12 bits from djpeg's 12-bit decode of
  corpus/primary/testorig12.jpg, and CMYK copies with the transform byte
  (file offset 17) set to 0
- `tw-<stem>-<variant>`: jpegtran twins (arithmetic, progressive, restart
  every MCU row, progressive with a restart every four MCUs) of the thirteen
  4-component corpus originals; each decodes to exactly its original's
  output, proving the entropy decoders on four components
- `hdr-*`: header edits pinning the colour-space rules: a 4-component file
  with its Adobe marker removed (CMYK), with JFIF added (still CMYK), with
  its transform swapped; a 3-component RGB file without Adobe, with JFIF,
  with transform 1; testorig without JFIF, with Adobe transform 1, with
  component ids R G B and with ids 0 1 2

Every file is checked to decode with both `djpeg -dct int -pnm` and
`djpeg -dct int -nosmooth -pnm`, exit status 0 and nothing on stderr; each
twin to give exactly its original's output, and each header variant to
give the source's output or not as the rule it pins says. Pillow, tjbench
and jpegtran are deterministic, so two runs write identical files. Only
corpus/synthetic-m7/*.jpg is deleted and rewritten.
"""

import glob
import os
import subprocess
import sys

from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "oracle", "libjpeg-turbo-3.2.0", "bin")
OUT = os.path.join(ROOT, "corpus", "synthetic-m7")
TMP = os.path.join(ROOT, "..", "tmp", "gen-m7")
X0, Y0 = 100, 60
ORIGINALS = [f"synthetic/{s}.jpg" for s in ("cmyk-pillow-q75-420", "cmyk-pillow-q90-444", "ycck-magick-q75-420", "ycck-magick-q90-444", "ycck-tj-q85-411",
                                             "ycck-tj-q85-420", "ycck-tj-q85-422", "ycck-tj-q85-440", "ycck-tj-q85-444")] + \
    ["wild/imagers/ycck.jpg", "wild/mozilla/jpg-cmyk-1.jpg", "wild/mozilla/jpg-cmyk-2.jpg", "wild/pillow/pil_sample_cmyk.jpg"]
JFIF = bytes.fromhex("FFE000104A46494600010100000100010000")
ADOBE1 = bytes.fromhex("FFEE000E41646F626500640000000001")
CROPS = [(1, 1), (2, 1), (1, 2), (3, 3), (5, 5), (7, 5), (8, 8), (9, 9), (15, 17), (16, 16), (17, 15), (33, 31), (64, 48)]

sys.path.insert(0, os.path.join(ROOT, "tools"))
import gen_m6  # noqa: E402

gen_m6.TMP, gen_m6.OUT = TMP, OUT


def fail(msg):
    raise SystemExit(f"gen_m7: {msg}")


def djpeg(path, flags=()):
    r = subprocess.run([os.path.join(BIN, "djpeg"), "-dct", "int", *flags, "-pnm", path], capture_output=True)
    if r.returncode or r.stderr:
        fail(f"{path}: djpeg {list(flags)} exit {r.returncode} {r.stderr!r}")
    return r.stdout


def check(path):
    djpeg(path)
    djpeg(path, ["-nosmooth"])


def pil(name, img, **kw):
    dst = os.path.join(OUT, name + ".jpg")
    img.save(dst, **kw)
    check(dst)


def tj(name, ppm, q, s, *extra):
    r = subprocess.run([os.path.join(BIN, "tjbench"), ppm, str(q), "-pixelformat", "cmyk", "-subsamp", s, *extra,
                        "-componly", "-benchtime", "0.01", "-warmup", "0", "-quiet"], capture_output=True)
    if r.returncode:
        fail(f"{name}: tjbench {extra} exit {r.returncode} {r.stderr!r}")
    dst = os.path.join(OUT, name + ".jpg")
    os.replace(os.path.join(os.path.dirname(ppm), f"{os.path.basename(ppm)[:-4]}_{s}_Q{q}.jpg"), dst)
    check(dst)
    return dst


def segments(d):
    i, out = 2, []
    while i + 4 <= len(d) and d[i] == 0xFF:
        m = d[i + 1]; n = int.from_bytes(d[i + 2:i + 4], "big")
        out.append((m, i, n))
        if m == 0xDA:
            break
        i += 2 + n
    return out


def adobe(d):
    return next(s for s in segments(d) if s[0] == 0xEE and d[s[1] + 4:s[1] + 9] == b"Adobe")


def cmyk_copy(src, name):
    d = bytearray(open(src, "rb").read())
    a = adobe(d)
    if a[1] != 2 or d[17] != 2:
        fail(f"{name}: Adobe layout {a}")
    d[17] = 0
    dst = os.path.join(OUT, name + ".jpg")
    open(dst, "wb").write(d)
    check(dst)


def hdr(name, src, data, same):
    dst = os.path.join(OUT, name + ".jpg")
    open(dst, "wb").write(data)
    check(dst)
    if (djpeg(dst) == djpeg(src)) != same:
        fail(f"{name}: output {'differs from' if same else 'equals'} {src}'s")


def main():
    os.makedirs(TMP, exist_ok=True)
    os.makedirs(OUT, exist_ok=True)
    for f in glob.glob(os.path.join(OUT, "*.jpg")):
        os.remove(f)
    ppm8 = os.path.join(TMP, "full8.ppm")
    open(ppm8, "wb").write(djpeg(os.path.join(ROOT, "corpus", "primary", "testorig.jpg")))
    rgb = Image.open(ppm8).convert("RGB")
    cmyk = rgb.convert("CMYK")
    sub = {"444": 0, "422": 1, "420": 2}
    for s in ("444", "422", "420"):
        for q in (50, 95):
            pil(f"pil-full-s{s}-q{q}", cmyk, quality=q, subsampling=sub[s])
    for cw, ch in CROPS:
        c = cmyk.crop((X0, Y0, X0 + cw, Y0 + ch))
        for s in ("420", "444"):
            pil(f"pil-c{cw}x{ch}-s{s}-q75", c, quality=75, subsampling=sub[s])
    pil("pil-full-s420-q100", cmyk, quality=100, subsampling=2)
    pil("pil-full-s420-q1", cmyk, quality=1, subsampling=2)
    pil("pil-full-s422-q75-opt", cmyk, quality=75, subsampling=1, optimize=True)
    pil("pil-full-s420-q75-prog", cmyk, quality=75, subsampling=2, progressive=True)
    pil("pil-full-s444-q75-prog", cmyk, quality=75, subsampling=0, progressive=True)
    pil("pil-full-s420-q75-r1", cmyk, quality=75, subsampling=2, restart_marker_rows=1)
    pil("pil-full-s420-q75-r3B", cmyk, quality=75, subsampling=2, restart_marker_blocks=3)
    for s in ("444", "422", "440", "420", "411", "441"):
        tj(f"tj-full-s{s}-q50", ppm8, 50, s)
    for cw, ch in [(1, 1), (7, 5), (17, 15), (33, 31)]:
        p = os.path.join(TMP, f"c{cw}x{ch}-8.ppm")
        rgb.crop((X0, Y0, X0 + cw, Y0 + ch)).save(p)
        for s in ("420", "444"):
            tj(f"tj-c{cw}x{ch}-s{s}-q85", p, 85, s)
    for v, extra in (("prog", ["-progressive"]), ("arith", ["-arithmetic"]), ("arith-prog", ["-arithmetic", "-progressive"]),
                     ("r1", ["-restart", "1"]), ("r2B", ["-restart", "2B"]), ("prog-r4B", ["-progressive", "-restart", "4B"])):
        tj(f"tj-full-s420-q85-{v}", ppm8, 85, "420", *extra)
    tj("tj-full-s444-q85-arith", ppm8, 85, "444", "-arithmetic")
    tj("tj-full-s420-q100", ppm8, 100, "420")
    tj("tj-full-s420-q1", ppm8, 1, "420")
    tj("tj-full-s411-q85-arith-prog", ppm8, 85, "411", "-arithmetic", "-progressive")
    src12 = gen_m6.source()
    full12 = gen_m6.crop(src12, (src12[0], src12[1]))
    for s in ("444", "420", "422", "411"):
        tj(f"tj12-full-s{s}-q85", full12, 85, s, "-precision", "12")
    for cw, ch in [(1, 1), (7, 5), (17, 15), (33, 31)]:
        tj(f"tj12-c{cw}x{ch}-s420-q85", gen_m6.crop(src12, (cw, ch)), 85, "420", "-precision", "12")
    for v, extra in (("prog", ["-progressive"]), ("arith", ["-arithmetic"]), ("arith-prog", ["-arithmetic", "-progressive"]), ("r1", ["-restart", "1"])):
        tj(f"tj12-full-s420-q85-{v}", full12, 85, "420", "-precision", "12", *extra)
    for src, name in (("tj12-full-s444-q85", "tj12-full-s444-q85-cmyk"), ("tj12-full-s420-q85-prog", "tj12-full-s420-q85-prog-cmyk"),
                      ("tj12-c17x15-s420-q85", "tj12-c17x15-s420-q85-cmyk"), ("tj12-full-s420-q85-arith", "tj12-full-s420-q85-arith-cmyk")):
        cmyk_copy(os.path.join(OUT, src + ".jpg"), name)
    for o in ORIGINALS:
        path = os.path.join(ROOT, "corpus", o)
        stem = os.path.basename(o)[:-4]
        for v, args in (("arith", ["-arithmetic"]), ("prog", ["-progressive"]), ("r1", ["-restart", "1"]), ("pr4B", ["-progressive", "-restart", "4B"])):
            gen_m6.twin(path, f"tw-{stem}-{v}", args)
    c4 = os.path.join(ROOT, "corpus", "synthetic", "cmyk-pillow-q90-444.jpg")
    # The ycck-tj-* corpus files decode to all-black output (K is 0 everywhere,
    # so every ink product is 0), which cannot tell YCCK from CMYK: the YCCK
    # variants start from the ImageMagick one.
    y4 = os.path.join(ROOT, "corpus", "synthetic", "ycck-magick-q90-444.jpg")
    rgbf = os.path.join(ROOT, "corpus", "wild", "imagers", "rgb.jpg")
    yccf = os.path.join(ROOT, "corpus", "primary", "testorig.jpg")

    def without(d, seg):
        return d[:seg[1]] + d[seg[1] + 2 + seg[2]:]

    def xform(d, v):
        b = bytearray(d); a = adobe(d); b[a[1] + 15] = v
        return bytes(b)

    d = open(c4, "rb").read(); a = adobe(d)
    hdr("hdr-cmyk-noadobe", c4, without(d, a), True)
    hdr("hdr-cmyk-jfif", c4, d[:a[1]] + JFIF + d[a[1]:], True)
    hdr("hdr-cmyk-as-ycck", c4, xform(d, 2), False)
    d = open(y4, "rb").read(); a = adobe(d)
    hdr("hdr-ycck-noadobe", y4, without(d, a), False)
    hdr("hdr-ycck-as-cmyk", y4, xform(d, 0), False)
    d = open(rgbf, "rb").read(); a = adobe(d)
    hdr("hdr-rgb-noadobe", rgbf, without(d, a), True)
    hdr("hdr-rgb-jfif", rgbf, d[:a[1]] + JFIF + d[a[1]:], False)
    hdr("hdr-rgb-xform1", rgbf, xform(d, 1), False)
    d = open(yccf, "rb").read()
    j = next(s for s in segments(d) if s[0] == 0xE0 and d[s[1] + 4:s[1] + 9] == b"JFIF\x00")
    nj = without(d, j)
    hdr("hdr-ycc-nojfif", yccf, nj, True)
    hdr("hdr-ycc-adobe1", yccf, nj[:2] + ADOBE1 + nj[2:], True)

    def ids(data, new):
        b = bytearray(data)
        sg = segments(b)
        sof = next(s for s in sg if s[0] in (0xC0, 0xC1, 0xC2))
        sos = next(s for s in sg if s[0] == 0xDA)
        old = [b[sof[1] + 10 + 3 * c] for c in range(3)]
        for c in range(3):
            b[sof[1] + 10 + 3 * c] = new[c]
        for k in range(b[sos[1] + 4]):
            b[sos[1] + 5 + 2 * k] = new[old.index(b[sos[1] + 5 + 2 * k])]
        return bytes(b)
    hdr("hdr-ycc-rgbids", yccf, ids(nj, [82, 71, 66]), False)
    hdr("hdr-ycc-ids012", yccf, ids(nj, [0, 1, 2]), True)
    n = len(glob.glob(os.path.join(OUT, "*.jpg")))
    print(f"gen_m7: {n} files in corpus/synthetic-m7")


if __name__ == "__main__":
    main()
