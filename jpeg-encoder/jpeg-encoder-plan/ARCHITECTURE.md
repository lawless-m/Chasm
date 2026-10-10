# JPEG encoder in Whackford: architecture

## Purpose

The encoder writes the sanitiser's output. The decoder produces pixels from
a hostile JPEG, or refuses it (a refusal means quarantine); the encoder
writes those pixels as a fresh baseline JPEG that every browser reads.
Nothing from the input reaches the output except the pixels, the EXIF
orientation (applied to the pixels) and a checked ICC profile.

Correctness is measured against libjpeg-turbo 3.2.0's `cjpeg` byte for byte
(TESTING.md), as the decoder is measured against its `djpeg`.

## The output profile

One fixed profile, with two settings:

- Baseline sequential DCT (SOF0), 8-bit, Huffman coded with the four
  standard tables of T.81 Annex K. No restart markers, no progressive, no
  optimised tables.
- Greyscale input gives one-component greyscale output. Every other input
  (YCbCr, RGB, CMYK, YCCK, 12-bit) reaches the encoder as 8-bit RGB and is
  written as YCbCr.
- **quality**, 1 to 100, default 85: the Annex K tables scaled as libjpeg's
  `jpeg_quality_scaling` and `jpeg_add_quant_table` (baseline: entries
  clamped to 1..255).
- **sample**, `420`, `422` or `444`, default `420`: the luma sampling
  factors 2x2, 2x1 or 1x1, chroma 1x1. Ignored for greyscale.
- Markers, in order: SOI; APP0 JFIF 1.01 with density unit 0 and density
  1:1 (cjpeg's defaults); APP2 ICC chunks when a profile is carried; DQT;
  SOF0; DHT; SOS; the scan; EOI. This is the order cjpeg writes.

Optimised Huffman tables (`cjpeg -optimize`, 5 to 10 % smaller files) can
be added later as a setting without changing anything else.

## The two programs

Both follow the decoder's protocol: options on stdin lines, paths natively
as `/file/<absolute path>`, a refusal is a trap `REFUSED CODE: message`
with exit status 1, any other trap is a bug.

**`encode.wack`**: PPM in, JPEG out; the cjpeg equivalent, and what the
byte-exact tests drive. stdin lines: input path, output path, then
optional `quality=Q`, `sample=S`, `icc=<path>` in any order. Input is P5
or P6 with maxval 255, or 4095 (scaled to 8 bits as cjpeg's `rdppm`:
`(v * 255 + maxval / 2) / maxval`). stdout: `ENCODED width height ncomp`.
`icc=` embeds that file's bytes as given (no checks: it is the test
harness's input, not hostile).

**`clean.wack`**: JPEG in, JPEG out, in one process. stdin lines: input
path, output path, then optional `quality=Q`, `sample=S`. In order:

1. The decoder's `source.load`, `markers.open`, `markers.walk`: structural
   refusals keep the decoder's codes.
2. `meta.walk`: its own read-only walk over the same bytes for the EXIF
   orientation and the ICC profile (SECURITY.md).
3. The decoder's `rows.setup` and `rows.decode`, rows going to the encoder
   instead of the PPM writer. 12-bit rows are scaled to 8 bits as above.
4. Orientation 1: rows stream straight into the encoder. Any other
   orientation: rows are collected into one image buffer (at most
   80 MP x 3 bytes = 240 MB, inside the decoder's 1 GB budget), turned by
   `orient`, then streamed into the encoder.

stdout: the decoder's `FRAME width height ncomp kind` line, then
`CLEANED width height orientation icc-bytes` (the output's dimensions,
after any turn).

## Layout

```
jpeg-encoder/
  jpeg/
    elimits.wack    the encoder's limits and refusal codes
    qtables.wack    Annex K quantisation tables, quality scaling, zigzag order
    htables.wack    Annex K Huffman tables; code and length lookups (jpeg_make_c_derived_tbl)
    ppmread.wack    P5/P6 reader, maxval 255 or 4095
    ccolour.wack    RGB to YCbCr in libjpeg's fixed point (jccolor rgb_ycc_convert)
    downsample.wack h2v1 and h2v2 with libjpeg's bias; edge replication (jcsample, jcprepct)
    fdct.wack       jfdctint, the integer forward DCT (cjpeg -dct int)
    quantise.wack   divide and round as jcdctmgr's C path
    bitwriter.wack  bit packing, FF to FF 00 stuffing, the last byte padded with 1 bits
    huffenc.wack    DC difference and AC run/size coding of one block (jchuff encode_one_block)
    writer.wack     the markers: SOI, APP0, APP2 ICC chunks, DQT, SOF0, DHT, SOS, EOI
    encoder.wack    the row sink: buffers one MCU row, then codes it
    meta.wack       EXIF orientation and ICC profile from the input
    orient.wack     the eight EXIF orientations applied to an image
  encode.wack       main: PPM to JPEG
  clean.wack        main: JPEG to JPEG
  tools/            harness.py (against cjpeg), the orientation reference, generators
  corpus/           synthetic inputs (EXIF and ICC cases); see TESTING.md
  docs/language-feedback.md
```

Programs are assembled from file lists in the harness, as the decoder's
are. `clean` lists the decoder's files from `../jpeg-decoder/jpeg/`
unchanged, then the encoder's.

## Data flow

The encoder is a row sink: `encoder.open ( path width height ncomp quality
sample icc -- enc )`, `encoder.row ( enc bytes -- )` taking one row of
packed 8-bit samples (1 or 3 bytes a pixel, the shape `rows.decode`
hands out), `encoder.close ( enc -- )`.

Each row is colour converted on arrival into per-component sample rows.
When an MCU row is buffered (16 pixel rows at 4:2:0, 8 at 4:2:2, 4:4:4 and
greyscale), the encoder replicates the right edge out to a whole MCU,
downsamples, then for each block runs the forward DCT, quantises and
Huffman codes it. At the bottom, the last pixel row is replicated down to a
whole MCU row. Output bytes go to the file in 1 MiB buffers. Memory is one
MCU row of samples and one output buffer, whatever the image height.

## The libjpeg behaviours to mirror

Each is a place where a correct-looking encoder differs from cjpeg by a
bit or a byte:

- Colour conversion: `rgb_ycc_convert`'s 16-bit fixed-point tables,
  including the `CBCR_OFFSET` and the `ONE_HALF - 1` rounding fudge on the
  chroma tables.
- Edge padding: `expand_right_edge` replicates the last column to the
  padded width before downsampling; `jcprepct` replicates the last row to
  the padded height.
- Downsampling: `h2v1_downsample` alternates bias 0,1; `h2v2_downsample`
  alternates 1,2. Full-size components are copied.
- Forward DCT: `jfdctint.c` (`jpeg_fdct_islow`), samples level-shifted by
  128 first, output scaled by 8.
- Quantisation: the C `quantize` in `jcdctmgr.c` with its reciprocal
  tables gives the same result as rounded division of the DCT output by
  8 x the table entry; the tests decide, and the reciprocal form is
  mirrored if they differ.
- Huffman: DC differences per component, reset at the start of the scan;
  EOB `0x00` and ZRL `0xF0` as jchuff emits them.
- Final byte: padded with 1 bits; every `FF` in entropy data followed by
  `00`.
- ICC: `jpeg_write_icc_profile` chunks of at most 65,519 bytes (65,533
  minus the 14-byte `ICC_PROFILE\0` + sequence + count header).

## Shared with the decoder

The decoder plan expects `markers`, `bits`, `colour`, `ppm` and the tables
to be shared. Until the M8 fuzz week has ended and E7 is done, nothing
under `jpeg-decoder/` changes: the encoder uses the decoder's files as they
are and writes its own mirror files. Merging into shared files is a step
of its own after E7, followed by a fresh decoder fuzz run.

## Decisions taken

- 2026-10-09: the oracle is cjpeg 3.2.0, byte for byte.
- 2026-10-09: one fixed output profile (baseline, standard Huffman tables,
  JFIF, 8-bit); quality and sampling are settings, default q85 4:2:0. A
  sample of 300 Brandbank T0 files was 92 % IJG q40 4:2:0, so the service
  is expected to set these per source.
- 2026-10-09: EXIF orientation is applied to the pixels; the ICC profile is
  carried after checks; everything else in the input is dropped.
- 2026-10-09: standard Huffman tables only; optimised tables are a later
  setting.
- 2026-10-09: `clean` is one process (decoder rows into the encoder), not
  two processes joined by a PPM pipe.
- 2026-10-10: the pieces both directions use live in jpeg-shared/ (limits, refuse, fixtures with hex and hex-array, dct: the natural order and DESCALE, out: the buffered output sink), listed first by every program. Direction-specific code (bit reader and writer, Huffman decoding and encoding, colour conversion and its inverse, the PPM writer and reader, up- and downsampling) stays in each program because the two directions differ. A directory of its own, so neither program owns the other's dependencies.
- 2026-10-11: orientation is applied by source stride: orient.row-into computes each output row's first source pixel and per-pixel step once (orient.start), copying whole rows for orientations 1 and 4; quantisation stays a rounded division per coefficient, because libjpeg-turbo's reciprocal multiply measured no faster in Whackford.
