"""Generate a deterministic fuzz corpus from corpus/primary.

Usage: python3 tools/mkfuzz.py [--seed N]

Reads every corpus/primary/*.jpg, deletes corpus/fuzz/mutants/*.jpg (and
nothing else), and writes mutants named <source stem>-<kind>-<index>.jpg
into corpus/fuzz/mutants/. One random.Random(seed) drives every choice, so
two runs with the same seed write byte-identical files.

Kinds, per source (a kind that does not apply to a source is skipped):

- trunc    8 cuts: random offsets, one inside the SOF and one inside a DHT
- flip     8 files with 1 to 4 random byte flips
- ff, zero 4 each: a run of 1 to 8 bytes set to FF or 00
- seglen   6: a random segment's length field set to 0, 1, 2, FFFF or random
- sofdim   6: width and/or height set to 0, 1, 16385, 65535 or 16384 x 16384
- dht      2: a DHT over-subscribed (first count 3), all 16 counts FF
- dhtsym   1: the first DC table's first symbol set to 16
- dhtfull  1: the first DHT rebuilt as one full table (two 1-bit codes, the
             second all ones)
- nf       2: the component count set to 0 and 5, length left as is
- samp     2: a sampling byte set to 00 and 55
- dupid    1: two component ids made equal (3-component files)
- tq       1: a quantisation table index set to 4
- sofkind  4: the SOF marker changed to C3, C5, CB, CF
- dropseg  3: the DQT, the first DHT, the SOF deleted
- dupsof   1: the SOF segment duplicated
- sos      6: Ns 0, Ns 5, Cs 99, Td 7, Se 70, Ss 5
- ent      8: 1 to 4 random byte flips inside the entropy-coded data
- entcut   4: the file cut inside the entropy-coded data
- dri      2: a DRI (interval 1, then 7) inserted before the SOS, with no
             RST markers in the data
- noeoi    1: the final EOI removed
- preeoi   2: 1 to 3 non-FF bytes inserted before the final EOI
- posteoi  1: 16 random bytes after the final EOI (allowed: still decodes)

Then corpus/wild/mozilla/jpg-progressive.jpg (32x32 4:2:0, libjpeg's
standard ten scans, a DHT before most of them) is the progressive source,
with stem jpg-progressive. Its kinds come after the primary loop and draw
from the same rng, so every earlier mutant is unchanged:

- pahal    6: a scan's Ah/Al byte edited (Ah = Al, a skipped level, Al 14,
             a refinement with Ah 0)
- pssse    6: Ss/Se edits (Ss > Se, Se 64, a DC band, an AC scan with three
             components, overlapping and incomplete bands)
- pdrop    4: scans 2, 4, 7, 10 deleted
- pdup     3: scans 1, 3, 10 duplicated in place
- pswap    3: scans (1,2), (2,3), (5,6) exchanged
- pent     8: 1 to 4 random bytes inside a later scan's data
- pcut     6: the file cut inside a later scan's data
- ptail    1: everything after scan 5 removed and an EOI appended (an
             incomplete, legal progressive file, which libjpeg smooths)
- pdht     2: the DHT before scan 3, then the one before scan 6, deleted
- pdqt     1: a DQT between scans (quant tables are latched, so the image
             is unchanged)
- pdri     2: a DRI (interval 1, then 7) before scan 2, no RST markers
- pnoeoi   1: the final EOI removed
- pseq     2: the SOF2 marker changed to C0, then C1

Then two arithmetic sources, again after everything above with the same
rng: corpus/primary/testimgari.jpg (stem testimgari, SOF9 with one DAC):

- adac     8: the DAC payload edited (L > U; L 0 U 15; Kx 0, 1, 63, 255;
             index 32; DC table 15)
- adaclen  3: the DAC length field set to 11 (odd), 2 (empty), 1
- adacdrop 1: the DAC removed (the defaults are the same values)
- adacdup  1: the DAC duplicated
- aent     8: 1 to 4 random byte values inside the data
- acut     6: the data cut at a random offset, FF D9 appended (libjpeg feeds
             zeros past a marker, so these decode)
- acutraw  4: the data cut at a random offset, nothing appended
- amark    3: FF D9, a DHT, FF D0 inserted at a random data offset
- adri     2: a DRI (interval 1, then 7) before the SOS, no RST markers
- akind    3: the SOF9 marker changed to CA, C1, C0
- anoeoi   1: the EOI removed
- aextra   2: a random non-zero byte, then two zero bytes, before the EOI

and corpus/synthetic-m5/a-full-s2x2-prog.jpg (stem a-full-s2x2-prog,
SOF10, ten scans, a DAC before each):

- pakind   2: SOF10 changed to C9, to C2
- padac    3: the DAC before scan 3 given L > U, given Kx 0, removed
- padrop   3: scans 2, 7, 10 deleted with their DACs (scan 7, a DC refine,
             has none)
- paswap   1: scans 1 and 2 exchanged, DACs included
- pacut    4: the file cut inside a later scan's data, FF D9 appended
- pacutraw 3: the same, nothing appended
- paent    6: 1 to 4 random bytes in a later scan's data
- pamark   2: FF D0 inside scan 3's data; a DHT before scan 5's DAC
- padri    1: a DRI with interval 3 before scan 2

Then two 12-bit sources, after everything above with the same rng:
corpus/primary/testorig12.jpg (stem testorig12, SOF1, P=12):

- tprec    4: the precision byte set to 8 (djpeg decodes it as 8-bit), 16,
             0, 13
- tkind    4: the SOF1 marker changed to C0 (decodes the same), C2, C9, CA
- tent     8: 1 to 4 random byte values inside the entropy data
- tcut     4: the file cut inside the entropy data
- tflip    6: 1 to 4 random byte flips anywhere
- tdqt16   2: the first DQT rewritten as a 16-bit table (entries x 17,
             capped at 65535; then every entry 0x0100)
- tdqt0    1: the first DQT's first entry set to 0
- tdri     2: a DRI (interval 1, then 7) before the SOS, no RST markers
- tnoeoi   1: the EOI removed
- tseglen  3: a random segment's length set to 0, FFFF, random
- tsofdim  2: width 0; 16384 x 16384

and corpus/synthetic/p12-prog-420.jpg (stem p12-prog-420, SOF2, P=12):

- pprec    4: the precision byte set to 8, 16, 0, 13
- pkind    3: SOF2 changed to C0, C1, CA
- pent     6: random bytes in a later scan's data
- pcut     4: the file cut inside a later scan's data
- pdrop    2: scan 2 deleted; the last scan deleted
- ptail    1: everything after scan 3 removed and FF D9 appended (an
             incomplete, legal 12-bit progressive file that libjpeg smooths)
- pdqt16   1: the first DQT rewritten as a 16-bit table (entries x 17)

Then two 4-component sources, after everything above with the same rng:
corpus/synthetic/cmyk-pillow-q75-420.jpg (CMYK, Adobe transform 0) and
corpus/synthetic/ycck-tj-q85-420.jpg (YCCK, transform 2; its output is
all black, so its decoded mutants test parsing, not colour), each with:

- xform    5: the Adobe transform byte set to 0, 1, 2, 3, 255 (libjpeg
             warns about 1, 3 and 255: refused BAD_COMPONENT)
- adobe    3: the APP14 removed; shortened to 11 payload bytes (ignored by
             libjpeg); its identifier changed to `Adobf` (ignored)
- jfif     1: a JFIF APP0 inserted before the APP14 (changes nothing for
             4 components)
- nf       2: the component count set to 3 with the fourth component's
             bytes and the length removed; set to 3 with nothing else
- ids      3: the ids changed consistently in the SOF and the SOS; the
             second made equal to the first; SOF ids R G B K with the SOS
             unchanged
- samp     3: the fourth component's sampling set to the first's (or 1x1),
             to 0x00, to 0x33
- ent      8: 1 to 4 random byte values inside the entropy data
- cut      4: the file cut inside the entropy data

The harness expects every fuzz file to be refused with a code, to reach
not-yet, or to decode byte-exact against the oracle; never to trap, hang or
mismatch.
"""

import argparse
import glob
import os
import random

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOFS = {0xC0, 0xC1, 0xC2, 0xC9, 0xCA}


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


def first(segs, pred):
    return next((s for s in segs if pred(s[0])), None)


def mutants(d, rng):
    segs = segments(d)
    sof = first(segs, lambda m: m in SOFS)
    dht = first(segs, lambda m: m == 0xC4)
    dqt = first(segs, lambda m: m == 0xDB)
    sos = first(segs, lambda m: m == 0xDA)
    out = []

    def put(kind, b):
        out.append((kind, bytes(b)))

    cuts = [rng.randrange(2, len(d)) for _ in range(6)]
    for s in (sof, dht):
        cuts.append(s[1] + rng.randrange(1, s[2] + 2) if s else rng.randrange(2, len(d)))
    for c in cuts:
        put("trunc", d[:c])
    for _ in range(8):
        b = bytearray(d)
        for _ in range(rng.randint(1, 4)):
            b[rng.randrange(len(b))] ^= rng.randrange(1, 256)
        put("flip", b)
    for kind, v in (("ff", 0xFF), ("zero", 0x00)):
        for _ in range(4):
            b = bytearray(d)
            at, n = rng.randrange(len(b)), rng.randint(1, 8)
            b[at : at + n] = bytes([v]) * len(b[at : at + n])
            put(kind, b)
    for v in (0, 1, 2, 0xFFFF, rng.randrange(65536), rng.randrange(65536)):
        b = bytearray(d)
        s = rng.choice(segs)
        b[s[1] + 2 : s[1] + 4] = v.to_bytes(2, "big")
        put("seglen", b)
    if sof:
        o = sof[1] + 4  # P, then Y at +1, X at +3, Nf at +5
        for w, h in ((0, None), (None, 0), (1, 1), (16385, None), (65535, 65535), (16384, 16384)):
            b = bytearray(d)
            if h is not None:
                b[o + 1 : o + 3] = h.to_bytes(2, "big")
            if w is not None:
                b[o + 3 : o + 5] = w.to_bytes(2, "big")
            put("sofdim", b)
        for v in (0, 5):
            b = bytearray(d)
            b[o + 5] = v
            put("nf", b)
        for v in (0x00, 0x55):
            b = bytearray(d)
            b[o + 7] = v
            put("samp", b)
        if d[o + 5] == 3:
            b = bytearray(d)
            b[o + 9] = b[o + 6]
            put("dupid", b)
        b = bytearray(d)
        b[o + 8] = 4
        put("tq", b)
        for m in (0xC3, 0xC5, 0xCB, 0xCF):
            b = bytearray(d)
            b[sof[1] + 1] = m
            put("sofkind", b)
        seg = d[sof[1] : sof[1] + 2 + sof[2]]
        put("dupsof", d[: sof[1] + 2 + sof[2]] + seg + d[sof[1] + 2 + sof[2] :])
    if dht:
        b = bytearray(d)
        b[dht[1] + 5] = 3
        put("dht", b)
        b = bytearray(d)
        b[dht[1] + 5 : dht[1] + 21] = b"\xff" * 16
        put("dht", b)
        if d[dht[1] + 4] >> 4 == 0:
            b = bytearray(d)
            b[dht[1] + 21] = 0x10
            put("dhtsym", b)
        o = dht[1]
        full = d[o + 4 : o + 5] + bytes([2] + [0] * 15) + d[o + 21 : o + 23]
        put("dhtfull", d[: o + 2] + (21).to_bytes(2, "big") + full + d[o + 2 + dht[2] :])
    for s in (dqt, dht, sof):
        if s:
            put("dropseg", d[: s[1]] + d[s[1] + 2 + s[2] :])
    if sos:
        o = sos[1] + 4  # Ns, then Cs/Td-Ta pairs, then Ss, Se, Ah/Al
        ns = d[o]
        edits = [(o, 0), (o, 5), (o + 1, 99), (o + 2, 0x70 | (d[o + 2] & 15)), (o + 2 + 2 * ns, 70), (o + 1 + 2 * ns, 5)]
        for at, v in edits:
            b = bytearray(d)
            b[at] = v
            put("sos", b)
    if sos and d[-2:] == b"\xff\xd9":
        start, end = sos[1] + 2 + sos[2], len(d) - 2
        for _ in range(8):
            b = bytearray(d)
            for _ in range(rng.randint(1, 4)):
                b[rng.randrange(start, end)] = rng.randrange(256)
            put("ent", b)
        for _ in range(4):
            put("entcut", d[: rng.randrange(start, end)])
        for v in (1, 7):
            dri = b"\xff\xdd\x00\x04" + v.to_bytes(2, "big")
            put("dri", d[: sos[1]] + dri + d[sos[1] :])
        put("noeoi", d[:-2])
        for _ in range(2):
            junk = bytes(rng.randrange(255) for _ in range(rng.randint(1, 3)))
            put("preeoi", d[:-2] + junk + d[-2:])
        put("posteoi", d + bytes(rng.randrange(256) for _ in range(16)))
    return out


def scans(d):
    """(SOS offset, header length, data start, data end) for every scan: the
    data ends at the first FF followed by neither 00 nor RSTn."""
    out = []
    i = 2
    while i + 4 <= len(d) and d[i] == 0xFF:
        m = d[i + 1]
        if m == 0xD9:
            break
        n = int.from_bytes(d[i + 2 : i + 4], "big")
        if m != 0xDA:
            i += 2 + n
            continue
        j = i + 2 + n
        while j + 1 < len(d) and not (d[j] == 0xFF and d[j + 1] != 0 and not 0xD0 <= d[j + 1] <= 0xD7):
            j += 1
        out.append((i, n, i + 2 + n, j))
        i = j
    return out


def progressive_mutants(d, rng):
    sc = scans(d)
    assert len(sc) == 10
    out = []

    def put(kind, b):
        out.append((kind, bytes(b)))

    def at(k):
        return sc[k - 1]

    def edit(k, field, v):
        b = bytearray(d)
        s0, n, start, _ = at(k)
        b[start + field] = v
        return b

    for k, v in ((2, 0x22), (6, 0x10), (6, 0x32), (7, 0x00), (1, 0x0E), (10, 0x21)):
        put("pahal", edit(k, -1, v))
    for k, field, v in ((2, -2, 0), (2, -2, 64), (2, -3, 0), (1, -2, 5), (5, -3, 2), (3, -2, 10)):
        put("pssse", edit(k, field, v))
    for k in (2, 4, 7, 10):
        s0, _, _, end = at(k)
        put("pdrop", d[:s0] + d[end:])
    for k in (1, 3, 10):
        s0, _, _, end = at(k)
        put("pdup", d[:end] + d[s0:end] + d[end:])
    for a, b in ((1, 2), (2, 3), (5, 6)):
        a0, _, _, a1 = at(a)
        b0, _, _, b1 = at(b)
        put("pswap", d[:a0] + d[b0:b1] + d[a1:b0] + d[a0:a1] + d[b1:])
    for _ in range(8):
        b = bytearray(d)
        _, _, start, end = at(rng.randint(2, 10))
        for _ in range(rng.randint(1, 4)):
            b[rng.randrange(start, end)] = rng.randrange(256)
        put("pent", b)
    for _ in range(6):
        _, _, start, end = at(rng.randint(2, 10))
        put("pcut", d[: rng.randrange(start, end)])
    put("ptail", d[: at(5)[3]] + b"\xff\xd9")
    for k in (3, 6):
        s0 = at(k)[0]
        prev_end = at(k - 1)[3]
        assert d[prev_end + 1] == 0xC4
        n = int.from_bytes(d[prev_end + 2 : prev_end + 4], "big")
        assert prev_end + 2 + n == s0
        put("pdht", d[:prev_end] + d[s0:])
    dqt = b"\xff\xdb\x00\x43\x00" + b"\x01" * 64
    put("pdqt", d[: at(4)[3]] + dqt + d[at(4)[3] :])
    for v in (1, 7):
        s0 = at(2)[0]
        put("pdri", d[:s0] + b"\xff\xdd\x00\x04" + v.to_bytes(2, "big") + d[s0:])
    put("pnoeoi", d[:-2])
    sof = d.index(b"\xff\xc2")
    for m in (0xC0, 0xC1):
        b = bytearray(d)
        b[sof + 1] = m
        put("pseq", b)
    return out


def arith_mutants(d, rng):
    segs = segments(d)
    dac = first(segs, lambda m: m == 0xCC)
    sof = first(segs, lambda m: m == 0xC9)
    sos = first(segs, lambda m: m == 0xDA)
    start, end = sos[1] + 2 + sos[2], len(d) - 2
    assert d[end:] == b"\xff\xd9"
    out = []

    def put(kind, b):
        out.append((kind, bytes(b)))

    p = dac[1] + 4  # the DAC payload: pairs (index, value)
    for at, v in ((p + 1, 0x12), (p + 1, 0xF0), (p + 5, 0), (p + 5, 1), (p + 5, 63), (p + 5, 255), (p, 0x20), (p, 0x0F)):
        b = bytearray(d)
        b[at] = v
        put("adac", b)
    for v in (0x0B, 0x02, 0x01):
        b = bytearray(d)
        b[dac[1] + 2 : dac[1] + 4] = v.to_bytes(2, "big")
        put("adaclen", b)
    seg = d[dac[1] : dac[1] + 2 + dac[2]]
    put("adacdrop", d[: dac[1]] + d[dac[1] + 2 + dac[2] :])
    put("adacdup", d[: dac[1]] + seg + d[dac[1] :])
    for _ in range(8):
        b = bytearray(d)
        for _ in range(rng.randint(1, 4)):
            b[rng.randrange(start, end)] = rng.randrange(256)
        put("aent", b)
    for _ in range(6):
        put("acut", d[: rng.randrange(start, end)] + b"\xff\xd9")
    for _ in range(4):
        put("acutraw", d[: rng.randrange(start, end)])
    for ins in (b"\xff\xd9", b"\xff\xc4\x00\x14\x00" + bytes(17), b"\xff\xd0"):
        at = rng.randrange(start, end)
        put("amark", d[:at] + ins + d[at:])
    for v in (1, 7):
        put("adri", d[: sos[1]] + b"\xff\xdd\x00\x04" + v.to_bytes(2, "big") + d[sos[1] :])
    for m in (0xCA, 0xC1, 0xC0):
        b = bytearray(d)
        b[sof[1] + 1] = m
        put("akind", b)
    put("anoeoi", d[:-2])
    put("aextra", d[:-2] + bytes([rng.randrange(1, 256)]) + d[-2:])
    put("aextra", d[:-2] + b"\x00\x00" + d[-2:])
    return out


def dac_before(d, sos_at):
    """(offset, length) of the DAC segment that ends right at sos_at, or
    (sos_at, 0) when there is none (a DC refine scan needs no DAC)."""
    i = d.rfind(b"\xff\xcc", 0, sos_at)
    n = int.from_bytes(d[i + 2 : i + 4], "big") if i >= 0 else 0
    return (i, n) if i >= 0 and i + 2 + n == sos_at else (sos_at, 0)


def arith_prog_mutants(d, rng):
    sc = scans(d)
    assert len(sc) == 10
    out = []

    def put(kind, b):
        out.append((kind, bytes(b)))

    def at(k):
        return sc[k - 1]

    def block(k):
        """The scan's DAC, header and data: (start, end)."""
        s0, _, _, end = at(k)
        return dac_before(d, s0)[0], end

    sof = d.index(b"\xff\xca")
    for m in (0xC9, 0xC2):
        b = bytearray(d)
        b[sof + 1] = m
        put("pakind", b)
    i, n = dac_before(d, at(3)[0])
    assert n == 4
    b = bytearray(d)
    b[i + 5] = 0x12
    put("padac", b)
    put("padac", d[: i + 4] + b"\x10\x00" + d[i + 6 :])
    put("padac", d[:i] + d[i + 2 + n :])
    for k in (2, 7, 10):
        a, e = block(k)
        put("padrop", d[:a] + d[e:])
    a1, e1 = block(1)
    a2, e2 = block(2)
    put("paswap", d[:a1] + d[a2:e2] + d[e1:a2] + d[a1:e1] + d[e2:])
    for _ in range(4):
        _, _, start, end = at(rng.randint(2, 10))
        put("pacut", d[: rng.randrange(start, end)] + b"\xff\xd9")
    for _ in range(3):
        _, _, start, end = at(rng.randint(2, 10))
        put("pacutraw", d[: rng.randrange(start, end)])
    for _ in range(6):
        b = bytearray(d)
        _, _, start, end = at(rng.randint(2, 10))
        for _ in range(rng.randint(1, 4)):
            b[rng.randrange(start, end)] = rng.randrange(256)
        put("paent", b)
    _, _, start, end = at(3)
    mid = (start + end) // 2
    put("pamark", d[:mid] + b"\xff\xd0" + d[mid:])
    i5 = dac_before(d, at(5)[0])[0]
    put("pamark", d[:i5] + b"\xff\xc4\x00\x14\x00" + bytes(17) + d[i5:])
    a2 = block(2)[0]
    put("padri", d[:a2] + b"\xff\xdd\x00\x04\x00\x03" + d[a2:])
    return out


def dqt16(d, seg, f):
    """The DQT segment seg rewritten with 16-bit entries f(x) (pq 1)."""
    _, at, n = seg
    body = d[at + 4 : at + 2 + n]
    out, j = bytearray(), 0
    while j < len(body):
        pq, tq = body[j] >> 4, body[j] & 15
        w = 2 if pq else 1
        vals = [int.from_bytes(body[j + 1 + w * k : j + 1 + w * k + w], "big") for k in range(64)]
        out += bytes([0x10 | tq]) + b"".join(min(65535, f(v)).to_bytes(2, "big") for v in vals)
        j += 1 + 64 * w
    return d[:at] + b"\xff\xdb" + (len(out) + 2).to_bytes(2, "big") + bytes(out) + d[at + 2 + n :]


def twelve_mutants(d, rng):
    segs = segments(d)
    sof = first(segs, lambda m: m == 0xC1)
    dqt = first(segs, lambda m: m == 0xDB)
    sos = first(segs, lambda m: m == 0xDA)
    start, end = sos[1] + 2 + sos[2], len(d) - 2
    assert d[end:] == b"\xff\xd9"
    out = []

    def put(kind, b):
        out.append((kind, bytes(b)))

    for v in (8, 16, 0, 13):
        b = bytearray(d)
        b[sof[1] + 4] = v
        put("tprec", b)
    for m in (0xC0, 0xC2, 0xC9, 0xCA):
        b = bytearray(d)
        b[sof[1] + 1] = m
        put("tkind", b)
    for _ in range(8):
        b = bytearray(d)
        for _ in range(rng.randint(1, 4)):
            b[rng.randrange(start, end)] = rng.randrange(256)
        put("tent", b)
    for _ in range(4):
        put("tcut", d[: rng.randrange(start, end)])
    for _ in range(6):
        b = bytearray(d)
        for _ in range(rng.randint(1, 4)):
            b[rng.randrange(len(b))] ^= rng.randrange(1, 256)
        put("tflip", b)
    put("tdqt16", dqt16(d, dqt, lambda v: v * 17))
    put("tdqt16", dqt16(d, dqt, lambda v: 0x0100))
    b = bytearray(d)
    b[dqt[1] + 5] = 0
    put("tdqt0", b)
    for v in (1, 7):
        put("tdri", d[: sos[1]] + b"\xff\xdd\x00\x04" + v.to_bytes(2, "big") + d[sos[1] :])
    put("tnoeoi", d[:-2])
    for v in (0, 0xFFFF, rng.randrange(65536)):
        b = bytearray(d)
        s = rng.choice(segs)
        b[s[1] + 2 : s[1] + 4] = v.to_bytes(2, "big")
        put("tseglen", b)
    o = sof[1] + 4
    b = bytearray(d)
    b[o + 3 : o + 5] = (0).to_bytes(2, "big")
    put("tsofdim", b)
    b = bytearray(d)
    b[o + 1 : o + 5] = (16384).to_bytes(2, "big") * 2
    put("tsofdim", b)
    return out


def twelve_prog_mutants(d, rng):
    segs = segments(d)
    sof = first(segs, lambda m: m == 0xC2)
    dqt = first(segs, lambda m: m == 0xDB)
    sc = scans(d)
    n = len(sc)
    assert n >= 4
    out = []

    def put(kind, b):
        out.append((kind, bytes(b)))

    for v in (8, 16, 0, 13):
        b = bytearray(d)
        b[sof[1] + 4] = v
        put("pprec", b)
    for m in (0xC0, 0xC1, 0xCA):
        b = bytearray(d)
        b[sof[1] + 1] = m
        put("pkind", b)
    for _ in range(6):
        b = bytearray(d)
        _, _, start, end = sc[rng.randint(1, n - 1)]
        for _ in range(rng.randint(1, 4)):
            b[rng.randrange(start, end)] = rng.randrange(256)
        put("pent", b)
    for _ in range(4):
        _, _, start, end = sc[rng.randint(1, n - 1)]
        put("pcut", d[: rng.randrange(start, end)])
    for k in (1, n - 1):
        s0, _, _, e = sc[k]
        put("pdrop", d[:s0] + d[e:])
    put("ptail", d[: sc[2][3]] + b"\xff\xd9")
    put("pdqt16", dqt16(d, dqt, lambda v: v * 17))
    return out


def four_mutants(d, rng):
    segs = segments(d)
    app = first(segs, lambda m: m == 0xEE)
    sof = first(segs, lambda m: m in (0xC0, 0xC1))
    sos = first(segs, lambda m: m == 0xDA)
    assert d[app[1] + 4 : app[1] + 9] == b"Adobe" and d[sof[1] + 9] == 4
    start, end = sos[1] + 2 + sos[2], len(d) - 2
    assert d[end:] == b"\xff\xd9"
    t = app[1] + 15
    o = sof[1] + 10  # first component spec: id, sampling, tq
    out = []

    def put(kind, b):
        out.append((kind, bytes(b)))

    for v in (0, 1, 2, 3, 255):
        b = bytearray(d)
        b[t] = v
        put("xform", b)
    put("adobe", d[: app[1]] + d[app[1] + 2 + app[2] :])
    b = bytearray(d)
    b[app[1] + 2 : app[1] + 4] = (13).to_bytes(2, "big")
    put("adobe", bytes(b[: app[1] + 2 + 13]) + d[app[1] + 2 + 14 :])
    b = bytearray(d)
    b[app[1] + 8] = ord("f")
    put("adobe", b)
    jfif = bytes.fromhex("FFE000104A46494600010100000100010000")
    put("jfif", d[: app[1]] + jfif + d[app[1] :])
    b = bytearray(d)
    b[sof[1] + 2 : sof[1] + 4] = (sof[2] - 3).to_bytes(2, "big")
    b[sof[1] + 9] = 3
    put("nf", bytes(b[: o + 9]) + d[o + 12 :])
    b = bytearray(d)
    b[sof[1] + 9] = 3
    put("nf", b)
    old = [d[o + 3 * c] for c in range(4)]
    new = [1, 2, 3, 4] if old != [1, 2, 3, 4] else [11, 12, 13, 14]
    b = bytearray(d)
    for c in range(4):
        b[o + 3 * c] = new[c]
    for k in range(d[sos[1] + 4]):
        b[sos[1] + 5 + 2 * k] = new[old.index(d[sos[1] + 5 + 2 * k])]
    put("ids", b)
    b = bytearray(d)
    b[o + 3] = b[o]
    put("ids", b)
    b = bytearray(d)
    for c, v in enumerate((82, 71, 66, 75)):
        b[o + 3 * c] = v
    put("ids", b)
    for v in (d[o + 1] if d[o + 10] != d[o + 1] else 0x11, 0x00, 0x33):
        b = bytearray(d)
        b[o + 10] = v
        put("samp", b)
    for _ in range(8):
        b = bytearray(d)
        for _ in range(rng.randint(1, 4)):
            b[rng.randrange(start, end)] = rng.randrange(256)
        put("ent", b)
    for _ in range(4):
        put("cut", d[: rng.randrange(start, end)])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--seed", type=int, default=1)
    a = ap.parse_args()
    rng = random.Random(a.seed)
    outdir = os.path.join(ROOT, "corpus", "fuzz", "mutants")
    os.makedirs(outdir, exist_ok=True)
    for f in glob.glob(os.path.join(outdir, "*.jpg")):
        os.remove(f)
    total = 0
    for src in sorted(glob.glob(os.path.join(ROOT, "corpus", "primary", "*.jpg"))):
        stem = os.path.splitext(os.path.basename(src))[0]
        d = open(src, "rb").read()
        counts = {}
        for kind, b in mutants(d, rng):
            counts[kind] = counts.get(kind, 0) + 1
            with open(os.path.join(outdir, f"{stem}-{kind}-{counts[kind]}.jpg"), "wb") as fh:
                fh.write(b)
            total += 1
    src = os.path.join(ROOT, "corpus", "wild", "mozilla", "jpg-progressive.jpg")
    counts = {}
    for kind, b in progressive_mutants(open(src, "rb").read(), rng):
        counts[kind] = counts.get(kind, 0) + 1
        with open(os.path.join(outdir, f"jpg-progressive-{kind}-{counts[kind]}.jpg"), "wb") as fh:
            fh.write(b)
        total += 1
    for path, stem, fn in ((("corpus", "primary", "testimgari.jpg"), "testimgari", arith_mutants),
                           (("corpus", "synthetic-m5", "a-full-s2x2-prog.jpg"), "a-full-s2x2-prog", arith_prog_mutants),
                           (("corpus", "primary", "testorig12.jpg"), "testorig12", twelve_mutants),
                           (("corpus", "synthetic", "p12-prog-420.jpg"), "p12-prog-420", twelve_prog_mutants),
                           (("corpus", "synthetic", "cmyk-pillow-q75-420.jpg"), "cmyk-pillow-q75-420", four_mutants),
                           (("corpus", "synthetic", "ycck-tj-q85-420.jpg"), "ycck-tj-q85-420", four_mutants)):
        counts = {}
        for kind, b in fn(open(os.path.join(ROOT, *path), "rb").read(), rng):
            counts[kind] = counts.get(kind, 0) + 1
            with open(os.path.join(outdir, f"{stem}-{kind}-{counts[kind]}.jpg"), "wb") as fh:
                fh.write(b)
            total += 1
    print(f"mkfuzz: {total} mutants in {os.path.relpath(outdir, ROOT)}")


if __name__ == "__main__":
    main()
