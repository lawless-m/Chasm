# Security

## What reaches the output

The output holds only: the decoded pixels (turned by the EXIF orientation),
the encoder's own markers and tables, and an ICC profile that passed the
checks below. No other input byte is copied. Strictness is preferred: a
refusal means quarantine, not a lost image.

The decoder's invariants (`../../jpeg-decoder/jpeg-decoder-plan/SECURITY.md`)
hold for `clean` unchanged, since it runs the decoder first. The encoder
adds:

- It never reads outside the input it was given, and the output size is
  bounded by the pixel count (at most a few bytes a pixel at q100).
- No allocation in loops; memory is one MCU row plus one output buffer,
  or the whole image when an orientation turn needs it (at most 240 MB).
- No `raw` words.

## EXIF orientation

`meta.walk` reads the first APP1 segment whose payload starts `Exif\0\0`.
Only the path to the orientation tag is checked:

- A TIFF header: `II*\0` or `MM\0*`, giving the byte order.
- The IFD0 offset, and its 2-byte entry count, inside the segment.
- All of IFD0's 12-byte entries inside the segment.
- Tag 0x0112, if present: type SHORT (3), count 1, value 1 to 8.

Any failure along that path refuses `BAD_EXIF`, an out-of-range
orientation included. No Exif APP1, or no tag 0x0112, means orientation 1.
Later Exif APP1 segments, the rest of IFD0, sub-IFDs, MakerNotes and
thumbnails are not read.

## ICC profile

APP2 segments whose payload starts `ICC_PROFILE\0` carry the profile in
chunks, each with a sequence number and a count. Following libjpeg's
`jpeg_read_icc_profile`, but refusing where it quietly ignores:

- Every chunk gives the same nonzero count; each sequence number is from 1
  to the count and appears exactly once; a chunk with no data after its
  header is refused. Otherwise `BAD_ICC`.
- The joined profile is at most `MAX_ICC` = 1 MB, or `LIMIT_ICC`.
- The joined profile is at least 128 bytes; its header size field (bytes
  0 to 3, big-endian) equals its length; bytes 36 to 39 are `acsp`; its tag
  count (bytes 128 to 131) and every 12-byte tag entry fit in the profile,
  and each tag's offset plus size is within the profile. Otherwise
  `BAD_ICC`.

The tag contents are not interpreted. A valid profile is carried only when
its data colour space (bytes 16 to 19) matches the output: `RGB ` for
colour, `GRAY` for greyscale. Any other valid profile (a CMYK profile on an
input converted to RGB, say) is dropped, not refused.

## Refusal codes

In `elimits.wack`, in the decoder's `REFUSED CODE: message` form:

| Code | Raised by | When |
|---|---|---|
| `BAD_EXIF` | `clean` | the path to the orientation tag is malformed, or its value is not 1 to 8 |
| `BAD_ICC` | `clean` | inconsistent chunks, or a malformed joined profile |
| `LIMIT_ICC` | `clean` | the joined profile is over 1 MB |
| `BAD_PPM` | `encode` | a bad PPM header, a maxval other than 255 or 4095, short data |

Every decoder code keeps its meaning in `clean`. Any other trap is a bug.
