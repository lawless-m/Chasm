"""Predict what clean does with a JPEG's metadata: the harness's independent
reference for clean's EXIF orientation, ICC profile and their refusals.

Usage: python3 -I tools/meta_ref.py [--grey] [--dump PATH] FILE

Prints `META <orientation> <icc-bytes>` (icc-bytes the carried profile's
length, 0 when none) and exits 0; or `REFUSED <CODE>` and exits 1; or
`UNPARSABLE` and exits 2 (the decoder refuses such a file first). --grey
says the output is greyscale (a colour output is assumed otherwise); --dump
writes the carried profile's bytes to PATH (an empty file when none).

This file is written from SECURITY.md alone, never from the Whackford
metadata reader, so that the two are independent. The rules:

The walk: the data starts FF D8; from offset 2 markers are read (FF fill
bytes skipped, then the marker byte) until SOS or EOI. A marker without a
length field (D8, 01, D0 to D7), or a length under 2 or running past the
data, makes the file unparsable. Every other marker has a big-endian
two-byte length that counts itself; its payload follows.

EXIF: the first APP1 whose payload starts `Exif\\0\\0` is read; later ones
are not. Its TIFF data (the payload after those six bytes) is at least 8
bytes and starts `II*\\0` (little-endian) or `MM\\0*` (big-endian); the
four-byte IFD0 offset at 4 (from the TIFF start, unsigned), its two-byte
entry count, and all of its 12-byte entries (tag, type, count, value) lie
inside the TIFF data. The first entry with tag 0x0112 has type 3 (SHORT),
count 1 and, as the first two bytes of its value field in the TIFF byte
order, a value of 1 to 8. Any failure along that path is BAD_EXIF. No Exif
APP1, or no tag 0x0112, is orientation 1. Nothing else is read.

ICC: every APP2 whose payload starts `ICC_PROFILE\\0` is a chunk: at least
one data byte after the 14-byte header (the 12-byte name, the sequence
number, the count); a nonzero count equal to every other chunk's; a
sequence number from 1 to the count, not seen before. Otherwise BAD_ICC,
where libjpeg's jpeg_read_icc_profile ignores the profile. After the walk:
every sequence number present (BAD_ICC); the joined profile at most
1000000 bytes (LIMIT_ICC, checked first); at least 128 bytes, its size
field (bytes 0 to 3, big-endian) equal to its length, `acsp` at 36, its
tag count (bytes 128 to 131) and every 12-byte tag entry from 132 inside
the profile, and each tag's offset plus size (unsigned) inside the profile
(BAD_ICC). Tag contents are not interpreted.

The profile is carried when its data colour space (bytes 16 to 19) is
`RGB ` for a colour output or `GRAY` for a greyscale one; any other valid
profile is dropped, not refused. With the output unknown (grey None) the
valid profile is returned whatever its space.

Refusals come in file order of the segment that fails, then the joined
profile's checks after the walk.
"""

import sys

MAX_ICC = 1000000


class Refused(Exception):
    pass


def segments(data):
    """(marker, payload) in file order up to SOS or EOI; None if unparsable."""
    if data[:2] != b"\xff\xd8":
        return None
    out, i = [], 2
    while True:
        if i >= len(data) or data[i] != 0xFF:
            return None
        while i < len(data) and data[i] == 0xFF:
            i += 1
        if i >= len(data):
            return None
        m = data[i]
        i += 1
        if m in (0xDA, 0xD9):
            return out
        if m in (0xD8, 0x01) or 0xD0 <= m <= 0xD7:
            return None
        if i + 2 > len(data):
            return None
        n = (data[i] << 8) | data[i + 1]
        if n < 2 or i + n > len(data):
            return None
        out.append((m, data[i + 2 : i + n]))
        i += n


def exif_orientation(tiff):
    if len(tiff) < 8:
        raise Refused("BAD_EXIF")
    if tiff[:4] == b"II*\x00":
        order = "little"
    elif tiff[:4] == b"MM\x00*":
        order = "big"
    else:
        raise Refused("BAD_EXIF")

    def u16(at):
        return int.from_bytes(tiff[at : at + 2], order)

    off = int.from_bytes(tiff[4:8], order)
    if off + 2 > len(tiff):
        raise Refused("BAD_EXIF")
    n = u16(off)
    if off + 2 + 12 * n > len(tiff):
        raise Refused("BAD_EXIF")
    for k in range(n):
        e = off + 2 + 12 * k
        if u16(e) == 0x0112:
            typ = u16(e + 2)
            count = int.from_bytes(tiff[e + 4 : e + 8], order)
            value = u16(e + 8)
            if typ != 3 or count != 1 or not 1 <= value <= 8:
                raise Refused("BAD_EXIF")
            return value
    return 1


def check_profile(p):
    if len(p) > MAX_ICC:
        raise Refused("LIMIT_ICC")
    if len(p) < 128 or int.from_bytes(p[0:4], "big") != len(p) or p[36:40] != b"acsp":
        raise Refused("BAD_ICC")
    if 132 > len(p):
        raise Refused("BAD_ICC")
    count = int.from_bytes(p[128:132], "big")
    if 132 + 12 * count > len(p):
        raise Refused("BAD_ICC")
    for k in range(count):
        e = 132 + 12 * k
        off = int.from_bytes(p[e + 4 : e + 8], "big")
        size = int.from_bytes(p[e + 8 : e + 12], "big")
        if off + size > len(p):
            raise Refused("BAD_ICC")


def predict(data, grey):
    """(code, orientation, profile): (None, o, p) on success, (code, None,
    None) on a refusal, (None, None, None) when unparsable."""
    segs = segments(data)
    if segs is None:
        return None, None, None
    try:
        orientation = 1
        seen_exif = False
        chunks, count = {}, None
        for m, pay in segs:
            if m == 0xE1 and not seen_exif and pay[:6] == b"Exif\x00\x00":
                seen_exif = True
                orientation = exif_orientation(pay[6:])
            elif m == 0xE2 and pay[:12] == b"ICC_PROFILE\x00":
                if len(pay) <= 14:
                    raise Refused("BAD_ICC")
                seq, n = pay[12], pay[13]
                if n == 0 or (count is not None and n != count) or not 1 <= seq <= n or seq in chunks:
                    raise Refused("BAD_ICC")
                count = n
                chunks[seq] = pay[14:]
        profile = None
        if chunks:
            if any(s not in chunks for s in range(1, count + 1)):
                raise Refused("BAD_ICC")
            p = b"".join(chunks[s] for s in range(1, count + 1))
            check_profile(p)
            space = p[16:20]
            if grey is None or (grey and space == b"GRAY") or (not grey and space == b"RGB "):
                profile = p
        return None, orientation, profile
    except Refused as e:
        return str(e), None, None


def main():
    args = sys.argv[1:]
    grey, dump = False, None
    while args and args[0].startswith("--"):
        if args[0] == "--grey":
            grey = True
            args = args[1:]
        elif args[0] == "--dump" and len(args) > 1:
            dump = args[1]
            args = args[2:]
        else:
            sys.exit(__doc__.splitlines()[3])
    if len(args) != 1:
        sys.exit(__doc__.splitlines()[3])
    code, orientation, profile = predict(open(args[0], "rb").read(), grey)
    if dump is not None:
        with open(dump, "wb") as f:
            f.write(profile or b"")
    if code:
        print(f"REFUSED {code}")
        sys.exit(1)
    if orientation is None:
        print("UNPARSABLE")
        sys.exit(2)
    print(f"META {orientation} {len(profile) if profile else 0}")


if __name__ == "__main__":
    main()
