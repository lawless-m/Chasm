# Milestones

Each milestone is a set of `declare`d words with `test` lines written
first. A milestone is done when its tests pass, its gate (TESTING.md)
passes with no `mismatch`, `trap`, `hang`, `cpu` or `memory`, and no `raw`
word has appeared. Nothing under `jpeg-decoder/` changes before E7 is done.

## E0: skeleton

- The `jpeg-encoder/` layout, `elimits.wack`, `ppmread.wack` (P5 and P6,
  maxval 255; `BAD_PPM`), `writer.wack` (SOI, APP0, DQT, SOF0, DHT, SOS,
  EOI), `bitwriter.wack`, `encode.wack`.
- `tools/harness.py` in `encode` mode, before any coding work.
- Gate: an 8x8 greyscale image of one flat value, at q 50, 85 and 100, is
  byte-exact.

## E1: greyscale

- `qtables.wack`, `fdct.wack`, `quantise.wack`, `htables.wack`,
  `huffenc.wack`, `encoder.wack` with one component, right and bottom edge
  replication.
- Gate: P5 inputs at every grid size and quality, byte-exact.

## E2: colour

- `ccolour.wack`, `downsample.wack`, interleaved MCUs, the `sample`
  setting.
- Gate: P6 inputs at every grid size, quality and sampling, and every
  8-bit corpus decode, byte-exact.

## E3: 12-bit input

- `ppmread` accepts maxval 4095 and rescales.
- Gate: the `p12-*` decodes (P5 and P6, maxval 4095) at every quality and
  sampling, byte-exact.

## E4: ICC on output

- `icc=<path>` in `encode`; APP2 chunking in `writer.wack`.
- Gate: byte-exact against `cjpeg -icc` with a small profile, a profile of
  exactly 65,519 bytes, and one needing three chunks.

## E5: clean, orientation 1

- `clean.wack`: the decoder's files, then rows into the encoder; 12-bit
  rows rescaled. No metadata yet: orientation 1, no ICC.
- Gate: `clean` over every corpus file the decoder accepts, at q 40 and 85
  and every sampling, byte-exact against `cjpeg (djpeg x)`; refusals match
  the decoder's own; `roundtrip` clean.

## E6: metadata

- `meta.wack`, `orient.wack`, `tools/orient.py`, `tools/gen_meta.py`,
  `BAD_EXIF`, `BAD_ICC`, `LIMIT_ICC`.
- Gate: every synthetic case either byte-exact against
  `cjpeg [-icc p] (rotate(djpeg x))` or refused with its expected code; the
  Brandbank sample byte-exact.

## E7: robustness

- The decoder's mutation fuzzer (`tools/mutate.py`) run against `clean`,
  under the same CPU and memory caps; findings to
  `jpeg-encoder/corpus/regressions/`.
- Gate: one hour clean on the full machine, after the M8 week has ended.

## After E7

- Merge the shared pieces (markers, bit I/O, tables, colour, PPM) into
  files both directions use; fuzz the decoder again before it ships.
- A speed pass, separately, byte-exact throughout.
- Optimised Huffman tables as a setting, if file size matters.
