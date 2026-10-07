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

The harness expects every fuzz file to be refused with a code or to reach
not-yet, never to trap or hang.
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
    print(f"mkfuzz: {total} mutants in {os.path.relpath(outdir, ROOT)}")


if __name__ == "__main__":
    main()
