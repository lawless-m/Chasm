"""Turn a binary PNM by an EXIF orientation: the reference clean's
orientation is checked against.

Usage: python3 -I tools/orient.py ORIENTATION IN OUT

IN is a binary P5 or P6 file as djpeg writes it: magic, width, height and
maxval separated by whitespace, one whitespace byte, then the samples, one
byte each at maxval 255, two big-endian bytes each at maxval 4095 (no other
maxval is accepted). OUT gets the same header with the turned width and
height (`P6\\n<w> <h>\\n<maxval>\\n`, or P5), then the turned samples.
Orientation 1 writes a copy.

Output pixel (x, y) is input pixel, W and H being the input's width and
height (orientations 5 to 8 swap the width and height):

    1 (x, y)            5 (y, x)
    2 (W-1-x, y)        6 (y, H-1-x)
    3 (W-1-x, H-1-y)    7 (W-1-y, H-1-x)
    4 (x, H-1-y)        8 (W-1-y, x)

These are the EXIF meanings: 6 means the stored image needs a 90 degree
clockwise turn to be upright, as Pillow's exif_transpose has it. The table
is checked against libjpeg-turbo's jpegtran, whose `-flip horizontal`,
`-rotate 180`, `-flip vertical`, `-transpose`, `-rotate 90`, `-transverse`
and `-rotate 270` are orientations 2 to 8 in that order. On an MCU-aligned
image its lossless transform decodes to exactly these pixels for
orientations 1 to 4 (greyscale, or colour decoded with -nosmooth); for 5 to
8, and for colour flips under fancy upsampling, the decodes differ by at
most 2 in a sample, since libjpeg's integer IDCT rounds its two passes
differently and fancy upsampling's rounding bias depends on position.

An orientation outside 1 to 8, a bad header, another maxval or short data
exits non-zero with a message and writes nothing.
"""

import sys


def dims(orientation, w, h):
    """The turned width and height."""
    return (h, w) if orientation >= 5 else (w, h)


def source(orientation, x, y, w, h):
    """The input pixel output pixel (x, y) comes from."""
    return {
        1: (x, y),
        2: (w - 1 - x, y),
        3: (w - 1 - x, h - 1 - y),
        4: (x, h - 1 - y),
        5: (y, x),
        6: (y, h - 1 - x),
        7: (w - 1 - y, h - 1 - x),
        8: (w - 1 - y, x),
    }[orientation]


def turn(orientation, w, h, bpp, data):
    """The image turned: bpp bytes a pixel, data exactly w * h * bpp bytes."""
    if len(data) != w * h * bpp:
        raise ValueError("data length")
    w2, h2 = dims(orientation, w, h)
    out = bytearray(len(data))
    o = 0
    for y in range(h2):
        for x in range(w2):
            sx, sy = source(orientation, x, y, w, h)
            i = (sy * w + sx) * bpp
            out[o : o + bpp] = data[i : i + bpp]
            o += bpp
    return bytes(out)


def read_pnm(path):
    d = open(path, "rb").read()
    fields, i = [], 0
    while len(fields) < 4:
        while i < len(d) and d[i : i + 1].isspace():
            i += 1
        j = i
        while j < len(d) and not d[j : j + 1].isspace():
            j += 1
        if j == i:
            raise ValueError("header ends early")
        fields.append(d[i:j])
        i = j
    magic = fields[0]
    if magic not in (b"P5", b"P6"):
        raise ValueError("not P5 or P6")
    w, h, maxval = int(fields[1]), int(fields[2]), int(fields[3])
    if maxval not in (255, 4095):
        raise ValueError(f"maxval {maxval}")
    bpp = (1 if magic == b"P5" else 3) * (1 if maxval == 255 else 2)
    data = d[i + 1 : i + 1 + w * h * bpp]
    if len(data) != w * h * bpp:
        raise ValueError("short data")
    return magic, w, h, maxval, bpp, data


def main():
    if len(sys.argv) != 4:
        sys.exit(__doc__.splitlines()[2])
    try:
        orientation = int(sys.argv[1])
        if not 1 <= orientation <= 8:
            raise ValueError(f"orientation {orientation}")
        magic, w, h, maxval, bpp, data = read_pnm(sys.argv[2])
    except (ValueError, OSError) as e:
        sys.exit(f"orient: {e}")
    w2, h2 = dims(orientation, w, h)
    with open(sys.argv[3], "wb") as f:
        f.write(b"%s\n%d %d\n%d\n" % (magic, w2, h2, maxval))
        f.write(turn(orientation, w, h, bpp, data))


if __name__ == "__main__":
    main()
