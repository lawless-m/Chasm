# Language feedback

What Whackford makes awkward, slow or impossible while writing the decoder.
Each entry says how things stand now and what the decoder does about it.

## Byte access

- `bytes` has no big-endian or 16-bit read: only `bytes.at`, `bytes.u32-at`
  and `bytes.u64-at`, all little-endian. Every 16-bit JPEG field (segment
  lengths, dimensions, 16-bit quantisation entries) is built from two
  `bytes.at`, and so is every coefficient of the whole-image store
  (`coeffs.get`, `coeffs.set`), with `i32.extend16_s` for the sign.
- There is no `bytes` literal, so tests build buffers from hex strings with a
  helper (`hex` in `jpeg/fixtures.wack`).

## Files

- There is no way to learn a file's size before reading it, so the loader
  allocates `MAX_INPUT_BYTES` + 1 bytes up front and reads into it.

## Refusals

- There are no exceptions and no catch: a refusal is a `trap`. A word that
  merely calls a word that always traps still owes its declared outputs
  (`E_EFFECT_MISMATCH`), so every refusal site is written
  `CODE "message" refusal trap`, with the primitive `trap` ending the branch.

## Control flow

- The short-circuit `and` and `or` take exactly two quotations, so three or
  more conditions do not chain (`[ a ] [ b ] or [ c ] or` is `E_SYNTAX`).
  Range checks that need no guard are written with `i32.or` on computed
  values instead.
- A `times` body must leave the stack as it found it, so a loop cannot
  gather eight values for a word that takes eight, or hand out eight
  results: the IDCT's eight inputs and outputs per column and per row are
  written out one by one (`idct.block`).
- A word with eight inputs (`upsample.fancy` takes a plane, a method, two
  neighbour offsets, a row count, a destination, its offset and a width)
  binds them all to locals on its first line; there is no record of named
  arguments short of declaring a struct for the call.

## Types

- A stack assertion asserts the whole stack, so `n array.new ( array i32 )`
  only works with nothing else on the stack: bind the array to a local first.

- An `array` of a struct type starts with unset elements and there is no test
  for one, so code that fills some slots keeps a parallel `array i32` of
  flags (`huffman.scan-tables`).

- `bytes` and `array i32` are different types with no shared accessor, so
  the 12-bit IDCT, upsampling and colour conversion are copies of the 8-bit
  words over `array i32` (idct.block12, upsample.*12, colour.row12); a
  16-bit `bytes` accessor would let one buffer type serve both precisions,
  at the cost of the 8-bit path's single-byte loads.

## Tooling

- `wack test` exits 0 even with pending tests, so gates grep the summary line
  for `0 failed, 0 pending`.
- `wack run` recompiles the program on every invocation, and there is no
  native runner for a built module without wasmtime, so the harness pays a
  compile per corpus file.

## Speed

Measured with the release compiler (`../target/release/wack run`, wasmtime,
bounds checks on), median of three runs, all in one session, on files made
with the oracle's cjpeg from a tiling of `primary/testorig.jpg`, 4000x3000
(12 megapixels), 4:2:0: `tmp/m2-timing/big12.jpg` (baseline),
`tmp/m4-timing/big12p.jpg` (progressive, libjpeg's standard ten scans),
`big12s3.jpg` (three one-component sequential scans),
`tmp/m5-timing/big12a.jpg` and `big12pa.jpg` (`cjpeg -arithmetic`,
sequential and progressive), and `tmp/m6-timing/big12b12.jpg` (`cjpeg
-precision 12` of the same 8-bit picture, SOF1). Every output is byte-exact
against djpeg in the same mode (`-dct int -pnm` for fancy, `-dct int
-nosmooth -pnm` for plain).

| File | Mode | `wack run` | `wack run --opt` | Decode rate | Oracle |
|---|---|---|---|---|---|
| big12, baseline | fancy (default) | 0.90 s | 0.81 s | 15.8 megapixels/s | 0.08 s |
| big12, baseline | plain (`nosmooth`) | 0.95 s | 0.74 s | 14.8 megapixels/s | 0.06 s |
| big12p, progressive | fancy (default) | 1.38 s | 1.10 s | 9.6 megapixels/s | 0.13 s |
| big12p, progressive | plain (`nosmooth`) | 1.40 s | 1.03 s | 9.5 megapixels/s | 0.11 s |
| big12s3, three scans | fancy (default) | 1.11 s | 0.82 s | 12.4 megapixels/s | 0.08 s |
| big12s3, three scans | plain (`nosmooth`) | 1.12 s | 0.81 s | 12.3 megapixels/s | 0.07 s |
| big12a, arithmetic | fancy (default) | 1.56 s | 0.99 s | 8.5 megapixels/s | 0.23 s |
| big12a, arithmetic | plain (`nosmooth`) | 1.51 s | 1.03 s | 8.8 megapixels/s | 0.21 s |
| big12pa, progressive arithmetic | fancy (default) | 1.64 s | 1.07 s | 8.0 megapixels/s | 0.26 s |
| big12pa, progressive arithmetic | plain (`nosmooth`) | 1.58 s | 1.06 s | 8.3 megapixels/s | 0.24 s |
| big12b12, 12-bit | fancy (default) | 1.66 s | 1.10 s | 7.9 megapixels/s | 0.14 s |
| big12b12, 12-bit | plain (`nosmooth`) | 1.61 s | 1.24 s | 8.1 megapixels/s | 0.12 s |

- Compiling the program is 0.14 s of every run (a file that stops at
  `NOT_YET` takes that long), well inside the harness's 20 s timeout.
- The oracle is libjpeg-turbo 3.2.0 in plain C without SIMD; the decoder
  runs 6 to 13 times slower than it (closest on arithmetic files, where the
  oracle itself is three times slower than on Huffman ones).
- The store path costs about 0.2 s on 12 megapixels for the same entropy
  work (big12s3 against big12): every coefficient goes into the store and
  back out through two `bytes.at` or two `bytes.at!` (there is no 16-bit
  accessor), and the IDCT runs in a second pass. Ten progressive scans cost
  another 0.27 s (big12p against big12s3), mostly entropy decoding of the
  refinement scans.
- Arithmetic decoding costs about 0.45 s more than Huffman decoding of the
  same picture through the same store (big12a against big12s3). A block
  takes about 52 binary decisions for 7 nonzero AC coefficients (counted on
  `primary/testimgari.jpg`); each decision reads and writes the arith
  record's c, a and ct fields, fetches and stores its statistics byte with
  `bytes.at` and `bytes.at!`, and indexes the state table, every access
  bounds-checked. `--opt` (Binaryen) recovers most of that.
- The 12-bit file decodes in 1.66 s against big12's 0.90 s. Its entropy data
  is 3.7 times larger (6.8 MB against 1.8 MB: the same quality setting keeps
  four more bits of every coefficient), and its samples live in `array i32`
  planes and upsampling buffers (4 bytes a sample against 1) and leave as
  two big-endian bytes; the oracle's time rises in the same proportion
  (0.14 s against 0.08 s). The 8-bit path pays one `decoder.wide` branch per
  block and per component row, which these measurements do not show.
- In a baseline decode the stages take, measured by removing one at a time
  in scratch copies: IDCT 27% of the run, colour conversion 23%, entropy
  decoding and the scan loop 18%, compile 12%, upsampling 10% and the row
  writes 10% (one `host.write` per output row). The fancy filters cost
  about 6% over replication. No stage dominates.
- The arithmetic decoder's registers live in a struct, so every decision
  costs several field reads and writes: a word cannot keep c, a and ct in
  locals across calls without returning three values, and a loop body
  cannot change the stack.
