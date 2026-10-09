"""Generate PPM test images for the encoder.

Usage: python3 -I tools/gen_ppm.py flat WIDTH HEIGHT VALUE OUT
       python3 -I tools/gen_ppm.py noise WIDTH HEIGHT SEED OUT
       python3 -I tools/gen_ppm.py pattern WIDTH HEIGHT SEED OUT

Each writes a binary P5 (greyscale, maxval 255) image of WIDTH x HEIGHT
pixels with the header `P5\\n<w> <h>\\n255\\n`, rows top to bottom:

- `flat`: every pixel VALUE (0 to 255); the E0 gate's 8x8 inputs.
- `noise`: `random.Random(SEED).randbytes(WIDTH * HEIGHT)`, deterministic
  for a seed; dense AC coefficients at every quality.
- `pattern`: smooth sinusoids, a rectangle with inverted contrast (sharp
  edges) and sparse seeded texture, clamped to 0..255; sparse AC
  coefficients at mid qualities (EOB and ZRL cases), dense ones at the
  edges.

`noise` and `pattern` are the `encode` inputs of the E1 gate: every grid
size at every grid quality.
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


def noise(a):
    size_ok(a)
    with open(a.out, "wb") as f:
        f.write(b"P5\n%d %d\n255\n" % (a.width, a.height))
        f.write(random.Random(a.seed).randbytes(a.width * a.height))


def pattern(a):
    size_ok(a)
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


def main():
    ap = argparse.ArgumentParser(description="Generate PPM test images for the encoder.")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("flat", help="a P5 image of one flat value")
    p.add_argument("width", type=int)
    p.add_argument("height", type=int)
    p.add_argument("value", type=int)
    p.add_argument("out")
    p.set_defaults(fn=flat)
    for name, fn, help in (("noise", noise, "seeded random bytes"), ("pattern", pattern, "seeded smooth content with edges")):
        p = sub.add_parser(name, help=help)
        p.add_argument("width", type=int)
        p.add_argument("height", type=int)
        p.add_argument("seed", type=int)
        p.add_argument("out")
        p.set_defaults(fn=fn)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
