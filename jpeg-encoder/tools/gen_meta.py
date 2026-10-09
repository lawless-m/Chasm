"""Generate the synthetic metadata corpus clean is tested on.

Usage: python3 -I tools/gen_meta.py [--base PPM] OUTDIR

Writes OUTDIR/<case>.jpg for every case below and OUTDIR/manifest.txt, one
line a file in sorted name order:

    <name>.jpg <grey|colour> match <orientation> <icc-bytes>
    <name>.jpg <grey|colour> refused <CODE>

the kind being the output's (grey for a greyscale frame). The output is
deterministic: no randomness, no timestamps, no network.

Bases: rgb is `cjpeg -dct int -baseline -quality 75 -sample 2x2` of the
base PPM (default ../tmp/dl/ljt-3.2.0/testimages/testorig.ppm, 227x149),
grey the same with -grayscale (the oracle's cjpeg); cmyk, p12 and p12grey
are copies of the decoder corpus's synthetic/cmyk-pillow-q75-420.jpg,
p12-seq-420.jpg and p12-grey.jpg. Regenerating needs the oracle and
tmp/dl's testorig.ppm; the outputs are committed, so nothing else does.

A case's segments go immediately after the first marker segment after SOI
(APP0 for cjpeg's files), in the order listed, unless said otherwise. The
Exif builder writes an APP1 of `Exif\\0\\0` and a TIFF: byte order `MM\\0*`
(or `II*\\0`), IFD0 offset 8, the entry count, the entries, a next-IFD 0;
the orientation entry is tag 0x0112, type 3, count 1, the SHORT in the
first two value bytes. The ICC builder makes a profile of a given length
(at least 144): the size big-endian at 0, the space at 16 (`RGB `, `GRAY`
or `CMYK`), `acsp` at 36, tag count 1 at 128, one entry at 132 (`desc`,
offset 144, size length - 144), bytes from 144 the pattern (i*7+3)&255;
chunked as cjpeg does (65519 data bytes a chunk, sequence 1 to k of k).

Cases (base rgb, colour, unless said):

    orient-1 .. orient-8        Exif with that orientation: match N 0
    orient-6-ii                 little-endian TIFF: match 6 0
    orient-6-grey               base grey: grey match 6 0
    orient-6-cmyk               base cmyk: match 6 0
    orient-6-p12                base p12: match 6 0
    orient-6-p12-grey           base p12grey: grey match 6 0
    orient-6-late               0x010E, 0x011A (offset to 8 extra bytes), then 0x0112 6: match 6 0
    orient-no-tag               IFD0 with 0x011A only: match 1 0
    orient-empty-ifd            entry count 0: match 1 0
    orient-second-app1          Exif 3, then Exif 6: match 3 0
    orient-xmp-first            an XMP APP1, then Exif 6: match 6 0
    orient-6-after-sof          Exif 6 just before SOS: match 6 0
    orient-6-and-icc            Exif 6, then a 300-byte RGB profile: match 6 300
    exif-bad-order              byte order XX\\0*: BAD_EXIF
    exif-short                  payload Exif\\0\\0MM\\0*: BAD_EXIF
    exif-empty                  payload Exif\\0\\0: BAD_EXIF
    exif-ifd-past               IFD0 offset 1000: BAD_EXIF
    exif-ifd-at-end             IFD0 offset at the TIFF's last byte: BAD_EXIF
    exif-count-past             count 50, one entry: BAD_EXIF
    exif-wrong-type             type 4: BAD_EXIF
    exif-wrong-count            count 2: BAD_EXIF
    exif-orient-0, -9           orientation 0, 9: BAD_EXIF
    exif-big-offset             IFD0 offset 0x80000008: BAD_EXIF
    exif-bad-truncated          orientation 0, file cut 100 bytes from its end: BAD_EXIF
    exif-bad-no-sof             SOI, APP0, orientation-0 Exif, EOI: NO_FRAME (the decoder's walk)
    icc-single                  300-byte RGB: match 1 300
    icc-multi                   150000 bytes, 3 chunks: match 1 150000
    icc-out-of-order            the same chunks in order 3, 1, 2: match 1 150000
    icc-exact-chunk             65519 bytes, one chunk: match 1 65519
    icc-two-chunks-min          65520 bytes: match 1 65520
    icc-after-sof               300 bytes just before SOS: match 1 300
    icc-min                     144 bytes: match 1 144
    icc-grey                    base grey, GRAY 300: grey match 1 300
    icc-gray-on-rgb             GRAY 300: match 1 0
    icc-rgb-on-grey             base grey, RGB 300: grey match 1 0
    icc-cmyk-on-cmyk            base cmyk, CMYK 300: match 1 0
    icc-cmyk-on-rgb             CMYK 300: match 1 0
    icc-chunk-missing           chunks 1 and 3 of 3: BAD_ICC
    icc-chunk-dup               chunks 1 and 1 of 2: BAD_ICC
    icc-count-mismatch          1 of 2, then 2 of 3: BAD_ICC
    icc-count-zero              1 of 0: BAD_ICC
    icc-seq-zero                0 of 1: BAD_ICC
    icc-seq-over                2 of 1: BAD_ICC
    icc-empty-chunk             one chunk, header only: BAD_ICC
    icc-empty-second            1 of 2 with data, 2 of 2 header only: BAD_ICC
    icc-header-short            ICC_PROFILE\\0 and one byte: BAD_ICC
    icc-size-wrong              size field + 1: BAD_ICC
    icc-no-acsp                 acsq: BAD_ICC
    icc-tiny                    100 bytes, correct size field: BAD_ICC
    icc-header-only             128 bytes, size and acsp correct: BAD_ICC
    icc-tag-table-past          tag count 1000: BAD_ICC
    icc-tag-past                tag size + 1000: BAD_ICC
    icc-tag-offset-wrap         tag offset 0xfffffff0: BAD_ICC
    icc-over-limit              1000001-byte RGB, 16 chunks: LIMIT_ICC
    meta-exif-then-icc-bad      orientation-0 Exif, then a 0-of-1 chunk: BAD_EXIF
    meta-icc-then-exif-bad      a 0-of-1 chunk, then orientation-0 Exif: BAD_ICC
    meta-icc-missing-exif-bad   chunk 1 of 2 only, then orientation-0 Exif: BAD_EXIF
"""

import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CJPEG = os.path.join(ROOT, "..", "jpeg-decoder", "oracle", "libjpeg-turbo-3.2.0", "bin", "cjpeg")
CORPUS = os.path.join(ROOT, "..", "jpeg-decoder", "corpus", "synthetic")
TESTORIG = os.path.join(ROOT, "..", "tmp", "dl", "ljt-3.2.0", "testimages", "testorig.ppm")
CHUNK = 65519


def seg(marker, payload):
    return bytes([0xFF, marker]) + (len(payload) + 2).to_bytes(2, "big") + payload


def after_first(base, segs):
    """Insert after the first marker segment that follows SOI."""
    n = int.from_bytes(base[4:6], "big")
    k = 4 + n
    return base[:k] + b"".join(segs) + base[k:]


def before_sos(base, segs):
    i = 2
    while True:
        m = base[i + 1]
        if m == 0xDA:
            return base[:i] + b"".join(segs) + base[i:]
        i += 2 + int.from_bytes(base[i + 2 : i + 4], "big")


def short(order, v):
    return v.to_bytes(2, order) + b"\x00\x00"


def tiff(entries, order="big", magic=None, offset=8, count=None, extra=b""):
    magic = magic or (b"MM\x00*" if order == "big" else b"II*\x00")
    body = (count if count is not None else len(entries)).to_bytes(2, order)
    for tag, typ, cnt, val in entries:
        body += tag.to_bytes(2, order) + typ.to_bytes(2, order) + cnt.to_bytes(4, order) + val
    return magic + offset.to_bytes(4, order) + body + b"\x00\x00\x00\x00" + extra


def exif(payload_tiff):
    return seg(0xE1, b"Exif\x00\x00" + payload_tiff)


def orient(n, order="big"):
    return exif(tiff([(0x0112, 3, 1, short(order, n))], order))


def profile(space, length):
    p = bytearray(length)
    p[0:4] = length.to_bytes(4, "big")
    p[16:20] = space
    p[36:40] = b"acsp"
    p[128:132] = (1).to_bytes(4, "big")
    p[132:144] = b"desc" + (144).to_bytes(4, "big") + (length - 144).to_bytes(4, "big")
    for i in range(144, length):
        p[i] = (i * 7 + 3) & 255
    return bytes(p)


def app2(seq, count, data):
    return seg(0xE2, b"ICC_PROFILE\x00" + bytes([seq, count]) + data)


def chunks(p):
    parts = [p[i : i + CHUNK] for i in range(0, len(p), CHUNK)]
    return [app2(k + 1, len(parts), d) for k, d in enumerate(parts)]


def cjpeg(ppm, grey):
    args = [CJPEG, "-dct", "int", "-baseline", "-quality", "75", "-sample", "2x2"]
    if grey:
        args.append("-grayscale")
    return subprocess.run(args + [ppm], check=True, capture_output=True).stdout


def cases(base):
    rgb, grey = base["rgb"], base["grey"]
    out = {}

    def add(name, data, kind, outcome):
        out[name] = (data, kind, outcome)

    for n in range(1, 9):
        add(f"orient-{n}", after_first(rgb, [orient(n)]), "colour", f"match {n} 0")
    add("orient-6-ii", after_first(rgb, [orient(6, "little")]), "colour", "match 6 0")
    add("orient-6-grey", after_first(grey, [orient(6)]), "grey", "match 6 0")
    add("orient-6-cmyk", after_first(base["cmyk"], [orient(6)]), "colour", "match 6 0")
    add("orient-6-p12", after_first(base["p12"], [orient(6)]), "colour", "match 6 0")
    add("orient-6-p12-grey", after_first(base["p12grey"], [orient(6)]), "grey", "match 6 0")
    rational = (72).to_bytes(4, "big") + (1).to_bytes(4, "big")
    late = tiff(
        [
            (0x010E, 2, 4, b"abc\x00"),
            (0x011A, 5, 1, (8 + 2 + 12 * 3 + 4).to_bytes(4, "big")),
            (0x0112, 3, 1, short("big", 6)),
        ],
        extra=rational,
    )
    add("orient-6-late", after_first(rgb, [exif(late)]), "colour", "match 6 0")
    notag = tiff([(0x011A, 5, 1, (8 + 2 + 12 + 4).to_bytes(4, "big"))], extra=rational)
    add("orient-no-tag", after_first(rgb, [exif(notag)]), "colour", "match 1 0")
    add("orient-empty-ifd", after_first(rgb, [exif(tiff([]))]), "colour", "match 1 0")
    add("orient-second-app1", after_first(rgb, [orient(3), orient(6)]), "colour", "match 3 0")
    xmp = seg(0xE1, b"http://ns.adobe.com/xap/1.0/\x00<x/>")
    add("orient-xmp-first", after_first(rgb, [xmp, orient(6)]), "colour", "match 6 0")
    add("orient-6-after-sof", before_sos(rgb, [orient(6)]), "colour", "match 6 0")
    add("orient-6-and-icc", after_first(rgb, [orient(6)] + chunks(profile(b"RGB ", 300))), "colour", "match 6 300")

    def bad_exif(name, t):
        add(name, after_first(rgb, [exif(t)]), "colour", "refused BAD_EXIF")

    good = [(0x0112, 3, 1, short("big", 1))]
    bad_exif("exif-bad-order", tiff(good, magic=b"XX\x00*"))
    add("exif-short", after_first(rgb, [seg(0xE1, b"Exif\x00\x00MM\x00*")]), "colour", "refused BAD_EXIF")
    add("exif-empty", after_first(rgb, [seg(0xE1, b"Exif\x00\x00")]), "colour", "refused BAD_EXIF")
    bad_exif("exif-ifd-past", tiff(good, offset=1000))
    t = tiff(good)
    bad_exif("exif-ifd-at-end", tiff(good, offset=len(t) - 1))
    bad_exif("exif-count-past", tiff(good, count=50))
    bad_exif("exif-wrong-type", tiff([(0x0112, 4, 1, (6).to_bytes(4, "big"))]))
    bad_exif("exif-wrong-count", tiff([(0x0112, 3, 2, short("big", 6))]))
    bad_exif("exif-orient-0", tiff([(0x0112, 3, 1, short("big", 0))]))
    bad_exif("exif-orient-9", tiff([(0x0112, 3, 1, short("big", 9))]))
    bad_exif("exif-big-offset", tiff(good, offset=0x80000008))
    o0 = orient(0)
    full = after_first(rgb, [o0])
    add("exif-bad-truncated", full[:-100], "colour", "refused BAD_EXIF")
    n0 = int.from_bytes(rgb[4:6], "big")
    add("exif-bad-no-sof", rgb[: 4 + n0] + o0 + b"\xff\xd9", "colour", "refused NO_FRAME")

    def icc(name, b, p_or_segs, kind, outcome, where=after_first):
        segs = p_or_segs if isinstance(p_or_segs, list) else chunks(p_or_segs)
        add(name, where(b, segs), kind, outcome)

    big = profile(b"RGB ", 150000)
    bigc = chunks(big)
    p300 = profile(b"RGB ", 300)
    icc("icc-single", rgb, p300, "colour", "match 1 300")
    icc("icc-multi", rgb, big, "colour", "match 1 150000")
    icc("icc-out-of-order", rgb, [bigc[2], bigc[0], bigc[1]], "colour", "match 1 150000")
    icc("icc-exact-chunk", rgb, profile(b"RGB ", 65519), "colour", "match 1 65519")
    icc("icc-two-chunks-min", rgb, profile(b"RGB ", 65520), "colour", "match 1 65520")
    icc("icc-after-sof", rgb, p300, "colour", "match 1 300", where=before_sos)
    icc("icc-min", rgb, profile(b"RGB ", 144), "colour", "match 1 144")
    icc("icc-grey", grey, profile(b"GRAY", 300), "grey", "match 1 300")
    icc("icc-gray-on-rgb", rgb, profile(b"GRAY", 300), "colour", "match 1 0")
    icc("icc-rgb-on-grey", grey, p300, "grey", "match 1 0")
    icc("icc-cmyk-on-cmyk", base["cmyk"], profile(b"CMYK", 300), "colour", "match 1 0")
    icc("icc-cmyk-on-rgb", rgb, profile(b"CMYK", 300), "colour", "match 1 0")

    parts = [big[i : i + CHUNK] for i in range(0, len(big), CHUNK)]

    def bad_icc(name, segs, code="BAD_ICC"):
        icc(name, rgb, segs, "colour", f"refused {code}")

    bad_icc("icc-chunk-missing", [app2(1, 3, parts[0]), app2(3, 3, parts[2])])
    bad_icc("icc-chunk-dup", [app2(1, 2, parts[0]), app2(1, 2, parts[1])])
    bad_icc("icc-count-mismatch", [app2(1, 2, parts[0]), app2(2, 3, parts[1])])
    bad_icc("icc-count-zero", [app2(1, 0, p300)])
    bad_icc("icc-seq-zero", [app2(0, 1, p300)])
    bad_icc("icc-seq-over", [app2(2, 1, p300)])
    bad_icc("icc-empty-chunk", [app2(1, 1, b"")])
    bad_icc("icc-empty-second", [app2(1, 2, p300), app2(2, 2, b"")])
    bad_icc("icc-header-short", [seg(0xE2, b"ICC_PROFILE\x00\x01")])
    p = bytearray(p300)
    p[0:4] = (301).to_bytes(4, "big")
    bad_icc("icc-size-wrong", [app2(1, 1, bytes(p))])
    p = bytearray(p300)
    p[36:40] = b"acsq"
    bad_icc("icc-no-acsp", [app2(1, 1, bytes(p))])
    tiny = bytearray(100)
    tiny[0:4] = (100).to_bytes(4, "big")
    tiny[16:20] = b"RGB "
    tiny[36:40] = b"acsp"
    bad_icc("icc-tiny", [app2(1, 1, bytes(tiny))])
    head = bytearray(128)
    head[0:4] = (128).to_bytes(4, "big")
    head[16:20] = b"RGB "
    head[36:40] = b"acsp"
    bad_icc("icc-header-only", [app2(1, 1, bytes(head))])
    p = bytearray(p300)
    p[128:132] = (1000).to_bytes(4, "big")
    bad_icc("icc-tag-table-past", [app2(1, 1, bytes(p))])
    p = bytearray(p300)
    p[140:144] = (300 - 144 + 1000).to_bytes(4, "big")
    bad_icc("icc-tag-past", [app2(1, 1, bytes(p))])
    p = bytearray(p300)
    p[136:140] = (0xFFFFFFF0).to_bytes(4, "big")
    bad_icc("icc-tag-offset-wrap", [app2(1, 1, bytes(p))])
    icc("icc-over-limit", rgb, profile(b"RGB ", 1000001), "colour", "refused LIMIT_ICC")

    seq0 = app2(0, 1, p300)
    add("meta-exif-then-icc-bad", after_first(rgb, [o0, seq0]), "colour", "refused BAD_EXIF")
    add("meta-icc-then-exif-bad", after_first(rgb, [seq0, o0]), "colour", "refused BAD_ICC")
    add("meta-icc-missing-exif-bad", after_first(rgb, [app2(1, 2, p300), o0]), "colour", "refused BAD_EXIF")
    return out


def main():
    args = sys.argv[1:]
    ppm = TESTORIG
    if len(args) == 3 and args[0] == "--base":
        ppm, args = args[1], args[2:]
    if len(args) != 1:
        sys.exit(__doc__.splitlines()[2])
    outdir = args[0]
    base = {
        "rgb": cjpeg(ppm, False),
        "grey": cjpeg(ppm, True),
        "cmyk": open(os.path.join(CORPUS, "cmyk-pillow-q75-420.jpg"), "rb").read(),
        "p12": open(os.path.join(CORPUS, "p12-seq-420.jpg"), "rb").read(),
        "p12grey": open(os.path.join(CORPUS, "p12-grey.jpg"), "rb").read(),
    }
    os.makedirs(outdir, exist_ok=True)
    lines = []
    for name, (data, kind, outcome) in sorted(cases(base).items()):
        with open(os.path.join(outdir, name + ".jpg"), "wb") as f:
            f.write(data)
        lines.append(f"{name}.jpg {kind} {outcome}")
    with open(os.path.join(outdir, "manifest.txt"), "w") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
