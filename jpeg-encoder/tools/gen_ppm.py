"""Generate PPM test images for the encoder.

Usage: python3 -I tools/gen_ppm.py flat WIDTH HEIGHT VALUE OUT
       python3 -I tools/gen_ppm.py noise WIDTH HEIGHT SEED OUT
       python3 -I tools/gen_ppm.py pattern WIDTH HEIGHT SEED OUT
       python3 -I tools/gen_ppm.py noise6 WIDTH HEIGHT SEED OUT
       python3 -I tools/gen_ppm.py pattern6 WIDTH HEIGHT SEED OUT

`flat`, `noise` and `pattern` write a binary P5 (greyscale, maxval 255)
image of WIDTH x HEIGHT pixels with the header `P5\\n<w> <h>\\n255\\n`;
`noise6` and `pattern6` write a P6 (RGB, maxval 255) image, three bytes a
pixel, with the header `P6\\n<w> <h>\\n255\\n`. Rows run top to bottom:

- `flat`: every pixel VALUE (0 to 255); the E0 gate's 8x8 inputs.
- `noise`: `random.Random(SEED).randbytes(WIDTH * HEIGHT)`, deterministic
  for a seed; dense AC coefficients at every quality.
- `pattern`: smooth sinusoids, a rectangle with inverted contrast (sharp
  edges) and sparse seeded texture, clamped to 0..255; sparse AC
  coefficients at mid qualities (EOB and ZRL cases), dense ones at the
  edges.

- `noise6`: `random.Random(SEED).randbytes(WIDTH * HEIGHT * 3)`.
- `pattern6`: `pattern` per channel with a different period for each of
  red, green and blue, the same inverted rectangle, and sparse seeded
  texture added to all three channels of a pixel.

`noise`, `pattern`, `noise6` and `pattern6` take `--maxval 4095` after the
subcommand (`gen_ppm.py noise6 --maxval 4095 W H SEED OUT`); the default is
255, and nothing else is accepted. At 4095 the header says 4095 and every
sample is two bytes, big-endian, the shape djpeg writes for a 12-bit
decode: `noise` and `noise6` draw each sample with
`random.Random(SEED).randrange(4096)`, row-major; `pattern` and `pattern6`
use 12-bit amplitudes (1024 on a base of 2048, inversion about 4095,
texture of -128..128) so the low four bits vary. The `--maxval 4095` noise
and pattern files are the E3 gate's generated inputs (P5 and P6 at several
grid sizes, every quality and sampling).

`noise` and `pattern` are the `encode` inputs of the E1 gate (every grid
size at every grid quality); `noise6` and `pattern6` those of the E2 gate
(every grid size at every grid quality and sampling).
"""

import argparse
import math
import random
import sys


def flat(a):
    if not 0 <= a.value <= 255:
        sys.exit(f"gen_ppm: VALUE must be 0 to 255, not {a.value}")
    if a.width < 1 or a.height < 1:
        sys.exit(f"gen_ppm: WIDTH and HEIGHT must be at least 1, not {a.width}x{a.height}")
    with open(a.out, "wb") as f:
        f.write(b"P5\n%d %d\n255\n" % (a.width, a.height))
        f.write(bytes([a.value]) * (a.width * a.height))


def size_ok(a):
    if a.width < 1 or a.height < 1:
        sys.exit(f"gen_ppm: WIDTH and HEIGHT must be at least 1, not {a.width}x{a.height}")


def write12(a, magic, n):
    """The 12-bit noise and pattern files: n samples a pixel, two bytes each."""
    w, h = a.width, a.height
    r = random.Random(a.seed)
    with open(a.out, "wb") as f:
        f.write(b"%s\n%d %d\n4095\n" % (magic, w, h))
        if a.fn in (noise, noise6):
            for y in range(h):
                f.write(b"".join(r.randrange(4096).to_bytes(2, "big") for _ in range(w * n)))
            return
        cx = [[int(1024 * math.sin(x / (9.0 + 3 * c))) for x in range(w)] for c in range(n)]
        cy = [[int(1024 * math.cos(y / (13.0 + 5 * c))) for y in range(h)] for c in range(n)]
        for y in range(h):
            row = bytearray(w * n * 2)
            inside_y = h // 4 <= y < h // 2
            for x in range(w):
                vs = [2048 + cx[c][x] + cy[c][y] for c in range(n)]
                if inside_y and w // 4 <= x < w // 2:
                    vs = [4095 - v for v in vs]
                if r.randrange(16) == 0:
                    vs = [v + r.randrange(-128, 129) for v in vs]
                for c in range(n):
                    v = min(4095, max(0, vs[c]))
                    row[2 * (n * x + c)] = v >> 8
                    row[2 * (n * x + c) + 1] = v & 0xFF
            f.write(bytes(row))


def noise(a):
    size_ok(a)
    if a.maxval == 4095:
        return write12(a, b"P5", 1)
    with open(a.out, "wb") as f:
        f.write(b"P5\n%d %d\n255\n" % (a.width, a.height))
        f.write(random.Random(a.seed).randbytes(a.width * a.height))


def pattern(a):
    size_ok(a)
    if a.maxval == 4095:
        return write12(a, b"P5", 1)
    w, h = a.width, a.height
    cx = [int(64 * math.sin(x / 9.0)) for x in range(w)]
    cy = [int(64 * math.cos(y / 13.0)) for y in range(h)]
    r = random.Random(a.seed)
    with open(a.out, "wb") as f:
        f.write(b"P5\n%d %d\n255\n" % (w, h))
        for y in range(h):
            row = bytearray(w)
            inside_y = h // 4 <= y < h // 2
            for x in range(w):
                v = 128 + cx[x] + cy[y]
                if inside_y and w // 4 <= x < w // 2:
                    v = 255 - v
                if r.randrange(16) == 0:
                    v += r.randrange(-8, 9)
                row[x] = 0 if v < 0 else 255 if v > 255 else v
            f.write(bytes(row))


def noise6(a):
    size_ok(a)
    if a.maxval == 4095:
        return write12(a, b"P6", 3)
    with open(a.out, "wb") as f:
        f.write(b"P6\n%d %d\n255\n" % (a.width, a.height))
        f.write(random.Random(a.seed).randbytes(a.width * a.height * 3))


def pattern6(a):
    size_ok(a)
    if a.maxval == 4095:
        return write12(a, b"P6", 3)
    w, h = a.width, a.height
    cx = [[int(64 * math.sin(x / (9.0 + 3 * c))) for x in range(w)] for c in range(3)]
    cy = [[int(64 * math.cos(y / (13.0 + 5 * c))) for y in range(h)] for c in range(3)]
    r = random.Random(a.seed)
    with open(a.out, "wb") as f:
        f.write(b"P6\n%d %d\n255\n" % (w, h))
        for y in range(h):
            row = bytearray(w * 3)
            inside_y = h // 4 <= y < h // 2
            for x in range(w):
                inv = inside_y and w // 4 <= x < w // 2
                vs = [128 + cx[c][x] + cy[c][y] for c in range(3)]
                if inv:
                    vs = [255 - v for v in vs]
                if r.randrange(16) == 0:
                    vs = [v + r.randrange(-8, 9) for v in vs]
                for c in range(3):
                    v = vs[c]
                    row[3 * x + c] = 0 if v < 0 else 255 if v > 255 else v
            f.write(bytes(row))


def main():
    ap = argparse.ArgumentParser(description="Generate PPM test images for the encoder.")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("flat", help="a P5 image of one flat value")
    p.add_argument("width", type=int)
    p.add_argument("height", type=int)
    p.add_argument("value", type=int)
    p.add_argument("out")
    p.set_defaults(fn=flat)
    for name, fn, help in (
        ("noise", noise, "seeded random bytes"),
        ("pattern", pattern, "seeded smooth content with edges"),
        ("noise6", noise6, "seeded random RGB bytes"),
        ("pattern6", pattern6, "seeded smooth RGB content with edges"),
    ):
        p = sub.add_parser(name, help=help)
        p.add_argument("--maxval", type=int, default=255)
        p.add_argument("width", type=int)
        p.add_argument("height", type=int)
        p.add_argument("seed", type=int)
        p.add_argument("out")
        p.set_defaults(fn=fn)
    a = ap.parse_args()
    if getattr(a, "maxval", 255) not in (255, 4095):
        sys.exit(f"gen_ppm: --maxval must be 255 or 4095, not {a.maxval}")
    a.fn(a)


if __name__ == "__main__":
    main()
