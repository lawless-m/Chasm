"""Generate PPM test images for the encoder.

Usage: python3 -I tools/gen_ppm.py flat WIDTH HEIGHT VALUE OUT

`flat` writes a binary P5 (greyscale, maxval 255) image of WIDTH x HEIGHT
pixels, every one VALUE (0 to 255), with the header `P5\\n<w> <h>\\n255\\n`.
These are the `encode` inputs for the E0 gate: an 8x8 image of one flat
value.
"""

import argparse
import sys


def flat(a):
    if not 0 <= a.value <= 255:
        sys.exit(f"gen_ppm: VALUE must be 0 to 255, not {a.value}")
    if a.width < 1 or a.height < 1:
        sys.exit(f"gen_ppm: WIDTH and HEIGHT must be at least 1, not {a.width}x{a.height}")
    with open(a.out, "wb") as f:
        f.write(b"P5\n%d %d\n255\n" % (a.width, a.height))
        f.write(bytes([a.value]) * (a.width * a.height))


def main():
    ap = argparse.ArgumentParser(description="Generate PPM test images for the encoder.")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("flat", help="a P5 image of one flat value")
    p.add_argument("width", type=int)
    p.add_argument("height", type=int)
    p.add_argument("value", type=int)
    p.add_argument("out")
    p.set_defaults(fn=flat)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
