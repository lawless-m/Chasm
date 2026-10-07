# Language feedback

What Whackford makes awkward, slow or impossible while writing the decoder.
Each entry says how things stand now and what the decoder does about it.

## Byte access

- `bytes` has no big-endian or 16-bit read: only `bytes.at`, `bytes.u32-at`
  and `bytes.u64-at`, all little-endian. Every 16-bit JPEG field (segment
  lengths, dimensions, 16-bit quantisation entries) is built from two
  `bytes.at`.
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

## Types

- A stack assertion asserts the whole stack, so `n array.new ( array i32 )`
  only works with nothing else on the stack: bind the array to a local first.

- An `array` of a struct type starts with unset elements and there is no test
  for one, so code that fills some slots keeps a parallel `array i32` of
  flags (`huffman.scan-tables`).

## Tooling

- `wack test` exits 0 even with pending tests, so gates grep the summary line
  for `0 failed, 0 pending`.
- `wack run` recompiles the program on every invocation, and there is no
  native runner for a built module without wasmtime, so the harness pays a
  compile per corpus file.

## Speed

Measured with the release compiler (`../target/release/wack run`, wasmtime,
bounds checks on), median of three runs, on baseline 4:2:0 files made by
tiling `primary/testorig.jpg` and compressing with the oracle's cjpeg
(`tmp/m2-timing/big3.jpg`, 2000x1500, 3 megapixels; `big12.jpg`,
4000x3000, 12 megapixels). Both decode byte-exact against djpeg.

| File | `wack run` | `wack run --opt` | Decode rate |
|---|---|---|---|
| big3, 3 MP | 0.40 s | 0.35 s | 11.6 megapixels/s |
| big12, 12 MP | 1.19 s | 0.88 s | 11.5 megapixels/s |

- Compiling the program is 0.14 s of every run (a file that stops at
  `NOT_YET` takes that long), well inside the harness's 20 s timeout; the
  decode itself is about 1.05 s for 12 MP.
- The oracle (libjpeg-turbo 3.2.0, plain C, no SIMD) decodes big12 in
  0.06 s, so the decoder runs about 17 times slower than C.
- Where the 1.15 s of a 12 MP run goes, measured by removing one stage at a
  time in scratch copies: IDCT 0.31 s, colour conversion 0.27 s, entropy
  decoding and the scan loop 0.21 s, compile 0.14 s, upsampling 0.12 s, and
  the row writes 0.11 s (one `host.write` per output row, 3000 for big12).
  No stage dominates.
