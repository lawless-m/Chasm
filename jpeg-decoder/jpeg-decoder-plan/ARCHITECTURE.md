# JPEG decoder in Whackford: architecture

## Purpose

A JPEG decoder written in Whackford, compiled to WebAssembly, to be the core of
a **safe image conversion sandbox**. Hostile input goes in; either correct
pixels or a clean, specific refusal comes out, and nothing else ever happens.

Correctness is measured against libjpeg-turbo byte for byte (see TESTING.md).
Safety is measured by the invariants in SECURITY.md.

A JPEG encoder follows later. Several modules are designed to be shared with it
(see "Shared with the encoder").

## Scope

In scope (ITU-T T.81 / ISO 10918-1):

- Baseline and extended sequential DCT, Huffman coded (SOF0, SOF1)
- Progressive DCT, Huffman coded (SOF2)
- Sequential and progressive DCT, arithmetic coded (SOF9, SOF10)
- 8-bit and 12-bit sample precision
- Greyscale, YCbCr, RGB, CMYK and YCCK; JFIF and Adobe APP14 conventions
- Any legal sampling factors (1 to 4 in each direction), including the odd
  ones (4:1:1, 4:4:0, 3:1 ...)
- Restart markers (DRI), DNL (define number of lines)
- Multiple quantisation and Huffman table definitions, tables redefined
  between scans

Out of scope, recognised and refused with a specific code:

- Lossless (SOF3, SOF11), hierarchical (SOF5-7, SOF13-15), differential modes
- JPEG-LS, JPEG 2000, JPEG XL, JPEG XT extensions
- Metadata pass-through of any kind (EXIF, ICC, XMP, comments, thumbnails):
  APPn and COM segments are parsed only as far as decoding needs them

## Deployment

Self-hosted, process per request:

1. A small front end (an HTTP handler or CGI, written in C# or Rust) receives
   the upload, writes it to a fresh per-request directory, and runs the
   decoder as a subprocess.
2. The decoder is the `wack build --wasi` module, run under wasmtime with the
   per-request directory preopened and wasmtime's own memory and fuel/time
   limits set (see SECURITY.md). Two sandbox layers: the language's checked
   memory model, and the wasm runtime's limits.
3. The decoder reads `/file/in.jpg`, writes `/file/out.ppm` (or the raw plane
   dump), prints a refusal line to stderr on failure, and exits 0 (decoded)
   or 1 (refused). A trap is exit 2 and is treated as a decoder bug to be
   fixed, though the front end treats it as a refusal.
4. The front end returns the result and deletes the directory. The wasm
   instance's memory is released with the process, which is why the language's
   never-free allocator is acceptable here.

During development `wack run` and `wack test` with `--mount` do the same job
with the ring host, and no front end is needed.

The decoder uses no processes or channels, so `build --wasi` is available.

The memory budget is a configuration constant, not a design assumption: a
Cloudflare Worker (128 MB per isolate) remains possible later by lowering it.

## Module layout

One library file per module, each with its own tests; a thin `main`.
Dependencies point downwards only.

```
jpeg/
  limits.wack       budget constants, pixel and dimension limits, refusal codes
  refuse.wack       the refusal interface: record a code and message, exit
  source.wack       load the input file into one bytes buffer
  markers.wack      segment walker; DQT, DHT, DAC, SOF, SOS, DRI, DNL, APP14
  frame.wack        frame/component geometry: sampling factors, MCU layout
  bits.wack         bit reader: stuffing, marker detection, end-of-data state
  huffman.wack      canonical table construction and symbol decode
  arith.wack        QM arithmetic decoder (Annex D), statistics bins
  entropy.wack      one interface over huffman and arith: decode a block/scan
  scan.wack         sequential scan decode: DC prediction, restart intervals
  progressive.wack  DC first/refine, AC first/refine, EOB runs, coefficient store
  coeffs.wack       16-bit packed coefficient store (progressive)
  idct.wack         libjpeg islow IDCT with dequantisation; 8 and 12 bit
  upsample.wack     fancy (triangle) upsampling matching libjpeg, and plain
  colour.wack       colour space decision tree and fixed-point conversions
  rows.wack         the pixel row sink interface and the MCU-row driver
  ppm.wack          PPM/PGM writer and raw plane dump, both row-streamed
  decoder.wack      ties it together; the public words
main.wack           CLI entry: paths, status, stderr refusal line
```

## Data flow

```
in.jpg -> source -> bytes
       -> markers: walk segments, fill tables, frame header, until SOS
       -> frame: check geometry, compute budget, allocate once
       -> for each scan:
            bits -> entropy (huffman|arith) -> scan|progressive
            sequential: per MCU row -> idct -> upsample -> colour -> rows -> ppm
            progressive: into coeffs store
       -> progressive only: after the last scan, per MCU row from coeffs
            -> idct -> upsample -> colour -> rows -> ppm
       -> EOI (or tolerated absence) -> exit 0
```

Nothing is allocated after the frame header has been checked against the
budget. The output image never exists in memory as a whole: `rows` hands one
row of pixels at a time to a sink.

## Key interfaces

**Refusal.** Every module refuses through one word that records a code from
`limits.wack` and a short message, then unwinds via `trap` with the message,
caught at `main` as exit 1. (Whackford has no exceptions; `trap` with a
recognisable prefix is the mechanism. `main` distinguishes a refusal trap
from a wasm trap by that prefix.) A bounds trap deep inside a loop is a bug:
the goal is that every malformed input is refused with a code at the point of
detection.

**Entropy decoder.** `scan` and `progressive` ask for decoded symbols without
knowing which coder is underneath: receive a DC difference category and its
extra bits, receive an AC run/size and extra bits, handle restart (reset DC
predictors and coder state). Huffman and arithmetic implement the same words.
The arithmetic coder's statistics-bin conditioning is per component and per
scan, so the coder state lives in a struct the scan owns.

**Pixel row sink.** `rows` produces one output row at a time: width, component
count, precision and the samples. `ppm` is the only consumer today; the
future encoder is the second. Keeping this boundary means the eventual
decode-and-re-encode path needs no intermediate file.

**Coefficient store.** Progressive decoding needs every coefficient resident
until the last scan. Stored 16-bit packed in a `bytes` buffer, two per `u32`
via `bytes.u32-at` unless the language gains `bytes.i16-at` / `bytes.i16-at!`
(recommended: it is a small primitive and halves the hottest inner loop).
Addressed by component, block row, block column, coefficient index.

## Memory budget

Computed from the frame header before any allocation:

- the input file (already loaded)
- progressive only: 2 bytes per coefficient; coefficients = sum over
  components of (blocks wide x blocks high x 64), padded to whole MCUs
- one MCU row of upsampled component planes
- one output row
- the fixed tables (Huffman lookup, quant, range-limit, colour tables)

If the total exceeds `MEM_BUDGET` (default 1 GB self-hosted; configurable),
or width or height exceed `MAX_DIM` (default 16384), or pixels exceed
`MAX_PIXELS` (default 80 MP), refuse before allocating. See SECURITY.md.

## Matching libjpeg-turbo exactly

The oracle is `djpeg -dct int -nosmooth`. To match it byte for byte:

- IDCT: `jpeg_idct_islow` (jidctint.c), including its CONST_BITS/PASS1_BITS
  rounding and the DESCALE behaviour. Not AAN, not floating point.
- Dequantisation: multiply then IDCT, as islow does; range-limit table of
  the same shape (the "sample range limit" of 5 x MAXJSAMPLE+1 entries).
- Upsampling: with `-nosmooth` libjpeg uses plain replication. Implement
  that first. Fancy upsampling (h2v1 and h2v2 triangle filters) second, so
  default djpeg output also matches; the test oracle switches accordingly.
- Colour: the fixed-point YCbCr to RGB tables from jdcolor.c (SCALEBITS 16,
  ONE_HALF rounding, the Cr_r / Cb_b / Cr_g / Cb_g tables).
- 12-bit: libjpeg-turbo 3.x's djpeg writes 12-bit PGM/PPM with maxval 4095;
  match that output (16-bit big-endian samples).

CMYK has no djpeg PPM oracle (djpeg refuses CMYK to PPM); see TESTING.md for
the raw-plane comparison against Pillow.

## Shared with the encoder

Designed as libraries from the start, used by both directions:

- `markers.wack` segment framing (the writer is the mirror)
- `bits.wack` has a sibling bit writer with byte stuffing
- zigzag tables, quantisation table handling, standard Huffman tables
- `colour.wack` in both directions
- `ppm.wack` gains a reader
- `rows.wack`: the encoder consumes the row sink

## Language changes worth considering

The project will stress Whackford; note anything that hurts in
`docs/language-feedback.md` as it comes up. Candidates already visible:

- `bytes.i16-at` / `bytes.i16-at!` / `bytes.u16-at` (coefficient store,
  12-bit samples)
- a way to learn a file's size before reading (today: read in chunks, or
  read the directory record); or a `read-bytes ( str -- bytes i32 )` prelude
  word that does it
- clamp helpers for `i32` (saturating to 0..255 / 0..4095) are hot; a
  prelude word or a range-limit table, whichever benchmarks better
- performance notes on bounds checks in the IDCT and colour loops after
  `wasm-opt`; `docs/performance.md` in the Whackford repo has the method

## Decisions

- **Strict by default.** Refuse non-conformant input except for a short
  allow-list of benign deviations (SECURITY.md). A sandbox that says "this
  file is malformed" is doing its job.
- **Zero `raw` words** in the decoder. `wack words` must show none outside
  the prelude. This is the audit.
- **Allocate once, after the budget check.** No `bytes.new`, `array.new`,
  `vec.make` or `map.make` inside per-block, per-MCU or per-row loops.
- **Integer arithmetic only** in the decode path. No `f64` anywhere between
  the bitstream and the output row, so results are deterministic and match
  libjpeg's integer path.
- **PPM out, raw planes for the harness.** P5 for greyscale, P6 for colour,
  maxval 255 or 4095. CMYK is converted to RGB for PPM and dumped raw for
  testing.
- **Lossless and hierarchical refused**, not implemented. Different codec,
  negligible real-world use in a conversion service.
