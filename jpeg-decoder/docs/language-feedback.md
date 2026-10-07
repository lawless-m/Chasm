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

## Tooling

- `wack test` exits 0 even with pending tests, so gates grep the summary line
  for `0 failed, 0 pending`.
- `wack run` recompiles the program on every invocation, and there is no
  native runner for a built module without wasmtime, so the harness pays a
  compile per corpus file.

## Speed

Measured with the release compiler (`../target/release/wack run`, wasmtime,
bounds checks on), median of three runs, all on the same day, on files made
with the oracle's cjpeg from a tiling of `primary/testorig.jpg`:
`tmp/m2-timing/big12.jpg` (4000x3000, 12 megapixels, baseline 4:2:0),
`tmp/m4-timing/big12p.jpg` and `big3p.jpg` (12 and 3 megapixels,
progressive with libjpeg's standard ten scans) and `big12s3.jpg` (12
megapixels, three one-component sequential scans). Every output is
byte-exact against djpeg in the same mode (`-dct int -pnm` for fancy,
`-dct int -nosmooth -pnm` for plain).

| File | Mode | `wack run` | `wack run --opt` | Decode rate | Oracle |
|---|---|---|---|---|---|
| big12, baseline | fancy (default) | 0.87 s | 0.79 s | 16.4 megapixels/s | 0.08 s |
| big12, baseline | plain (`nosmooth`) | 0.88 s | 0.70 s | 16.2 megapixels/s | 0.06 s |
| big12p, progressive | fancy (default) | 1.36 s | 0.99 s | 9.8 megapixels/s | 0.13 s |
| big12p, progressive | plain (`nosmooth`) | 1.34 s | 1.02 s | 10.0 megapixels/s | 0.12 s |
| big3p, progressive | fancy (default) | 0.37 s | 0.40 s | 13.1 megapixels/s | 0.04 s |
| big3p, progressive | plain (`nosmooth`) | 0.36 s | 0.43 s | 13.7 megapixels/s | 0.03 s |
| big12s3, three scans | fancy (default) | 1.06 s | 0.88 s | 13.0 megapixels/s | 0.08 s |
| big12s3, three scans | plain (`nosmooth`) | 1.07 s | 0.87 s | 12.9 megapixels/s | 0.07 s |

- Compiling the program is 0.14 s of every run (a file that stops at
  `NOT_YET` takes that long), well inside the harness's 20 s timeout.
- The oracle is libjpeg-turbo 3.2.0 in plain C without SIMD; the decoder
  runs 10 to 13 times slower than it.
- The store path costs 0.19 s on 12 megapixels for the same entropy work
  (big12s3 against big12): every coefficient goes into the store and back
  out through two `bytes.at` or two `bytes.at!` (there is no 16-bit
  accessor), and the IDCT runs in a second pass. Ten progressive scans cost
  another 0.30 s (big12p against big12s3), mostly entropy decoding of the
  refinement scans.
- In a baseline decode the stages take, measured by removing one at a time
  in scratch copies: IDCT 27% of the run, colour conversion 23%, entropy
  decoding and the scan loop 18%, compile 12%, upsampling 10% and the row
  writes 10% (one `host.write` per output row). The fancy filters cost
  about 6% over replication. No stage dominates.
