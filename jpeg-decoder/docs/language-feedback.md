# Language feedback

What Whackford makes awkward, slow or impossible while writing the decoder.
Each entry says how things stand now and what the decoder does about it.

## Byte access

- `bytes` has little-endian 16-, 32- and 64-bit reads and writes
  (`bytes.u16-at`, `bytes.u32-at`, `bytes.u64-at` and their `!` forms) but no
  big-endian read. Every 16-bit JPEG header field (segment lengths,
  dimensions, 16-bit quantisation entries) is built from two `bytes.at`; the
  whole-image coefficient store, which is little-endian, reads and writes
  each coefficient with one `bytes.u16-at` or `bytes.u16-at!`
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
  words over `array i32` (idct.block12, upsample.*12, colour.row12). With
  `bytes.u16-at` the 12-bit planes could be `bytes` at two bytes a sample,
  but one set of words for both precisions would still need the sample
  width passed in and a branch on it at every load and store.

- Choosing among five colour spaces is a nest of `if`s in colour.row and
  colour.row12 (grey, YCbCr, RGB, CMYK, YCCK), each new space one level
  deeper; there is no `case` on an integer.

## Tooling

- `wack test` exits 0 even with pending tests, so gates grep the summary line
  for `0 failed, 0 pending`.
- `wack run` compiles the Whackford program on every invocation, and there
  is no native runner for a built module without wasmtime. Wasmtime caches
  the compiled module, so the harness pays the Whackford compile per corpus
  file but not Cranelift's: a decode of a 1x1 file takes 0.08 s.

## Speed

Measured with the release compiler (`../target/release/wack run`, wasmtime
with its function inliner on, bounds checks on). Each row is the median of
three runs, and the table takes the per-row minimum of three such passes
(`tmp/p1/time.py`): on this machine a single median still varies by up to
0.3 s with background load. The files were made with the oracle's cjpeg
from a tiling of `primary/testorig.jpg`, 4000x3000 (12 megapixels), 4:2:0:
`tmp/m2-timing/big12.jpg` (baseline), `tmp/m4-timing/big12p.jpg`
(progressive, libjpeg's standard ten scans), `big12s3.jpg` (three
one-component sequential scans), `tmp/m5-timing/big12a.jpg` and
`big12pa.jpg` (`cjpeg -arithmetic`, sequential and progressive),
`tmp/m6-timing/big12b12.jpg` (`cjpeg -precision 12` of the same 8-bit
picture, SOF1), and `tmp/m7-timing/big12cmyk.jpg` (Pillow's CMYK of the
same picture, quality 75, Adobe transform 0, C subsampled 2x2 and M, Y, K
full size) and `big12ycck.jpg` (tjbench `-pixelformat cmyk`, quality 85,
Adobe transform 2, Y and K full size, Cb and Cr 2x2); djpeg writes both as
P6 RGB through cmyk_to_rgb. Every output is byte-exact against djpeg in the
same mode (`-dct int -pnm` for fancy, `-dct int -nosmooth -pnm` for plain).
The decode rate is 12 megapixels over the run less the startup, 0.08 s (a
`wack run` of a 1x1 file, compiled module cached). The oracle writes to
`/dev/null`; the decoder writes a real file.

| File | Mode | `wack run` | `wack run --opt` | Decode rate | Oracle |
|---|---|---|---|---|---|
| big12, baseline | fancy (default) | 0.49 s | 0.76 s | 28.8 megapixels/s | 0.08 s |
| big12, baseline | plain (`nosmooth`) | 0.47 s | 0.78 s | 30.4 megapixels/s | 0.06 s |
| big12p, progressive | fancy (default) | 0.76 s | 1.15 s | 17.7 megapixels/s | 0.13 s |
| big12p, progressive | plain (`nosmooth`) | 0.72 s | 0.93 s | 18.8 megapixels/s | 0.11 s |
| big12s3, three scans | fancy (default) | 0.54 s | 0.73 s | 26.2 megapixels/s | 0.08 s |
| big12s3, three scans | plain (`nosmooth`) | 0.60 s | 1.00 s | 23.1 megapixels/s | 0.06 s |
| big12a, arithmetic | fancy (default) | 0.74 s | 0.92 s | 18.3 megapixels/s | 0.22 s |
| big12a, arithmetic | plain (`nosmooth`) | 0.73 s | 0.94 s | 18.3 megapixels/s | 0.21 s |
| big12pa, progressive arithmetic | fancy (default) | 0.78 s | 1.01 s | 17.2 megapixels/s | 0.25 s |
| big12pa, progressive arithmetic | plain (`nosmooth`) | 0.72 s | 0.99 s | 18.5 megapixels/s | 0.24 s |
| big12b12, 12-bit | fancy (default) | 0.76 s | 0.97 s | 17.5 megapixels/s | 0.14 s |
| big12b12, 12-bit | plain (`nosmooth`) | 0.75 s | 1.32 s | 17.9 megapixels/s | 0.12 s |
| big12cmyk, CMYK | fancy (default) | 0.76 s | 0.76 s | 17.6 megapixels/s | 0.12 s |
| big12cmyk, CMYK | plain (`nosmooth`) | 0.51 s | 0.73 s | 27.9 megapixels/s | 0.10 s |
| big12ycck, YCCK | fancy (default) | 0.70 s | 0.87 s | 19.3 megapixels/s | 0.16 s |
| big12ycck, YCCK | plain (`nosmooth`) | 0.68 s | 0.87 s | 20.0 megapixels/s | 0.15 s |

- The oracle is libjpeg-turbo 3.2.0 in plain C without SIMD; the decoder
  runs 3 to 8 times slower than it, closest on arithmetic files, where the
  oracle itself is three times slower than on Huffman ones.
- The native engine has Cranelift's function inliner on (ARCHITECTURE.md
  section 28 item 3). Every struct field accessor and small helper word is
  a call in the wasm, so the inliner halves every row: big12 fancy takes
  0.50 s with it and 0.95 s without, big12a 0.75 s against 1.54 s
  (alternating runs, best wall time). `wack run --opt` (Binaryen -O3) is
  slower than plain `wack run` on every row: its inlining buys nothing the
  engine's does not, and its own time is paid on each run.
- Wall times hide the steadier CPU figures: by child CPU time, best of
  five, big12 fancy takes 0.38 s and big12cmyk fancy 0.45 s, where the
  table's wall times are 0.49 s and 0.76 s.
- In a baseline decode (big12 fancy, best of seven runs per variant, with
  one stage removed at a time in scratch copies, `tmp/p1/stages.py`) the
  stages take: IDCT 24% of the 0.46 s run, row writes 24%, upsampling 15%,
  startup 15%, entropy decoding and the scan loop 13%, colour conversion
  9%. The row writes go out in 1 MiB batches (`ppm.row` collects rows,
  `ppm.close` flushes), 35 `host.write` calls for the 36 MB image, so their
  cost is the kernel taking 36 MB into the file, not the calls: one write a
  row measured the same.
- The store path costs about 0.05 s on 12 megapixels for the same entropy
  work (big12s3 against big12): every coefficient goes into the store and
  back out through `bytes.u16-at!` and `bytes.u16-at`, and the IDCT runs in
  a second pass. Ten progressive scans cost another 0.22 s (big12p against
  big12s3), mostly entropy decoding of the refinement scans.
- Arithmetic decoding costs about 0.20 s more than Huffman decoding of the
  same picture through the same store (big12a against big12s3). A block
  takes about 52 binary decisions for 7 nonzero AC coefficients (counted on
  `primary/testimgari.jpg`); each decision reads and writes the arith
  record's c, a and ct fields, fetches and stores its statistics byte with
  `bytes.at` and `bytes.at!`, and indexes the state table, every access
  bounds-checked.
- The arithmetic decoder's registers live in a struct, so every decision
  costs several field reads and writes even with the accessors inlined: a
  word cannot keep c, a and ct in locals across calls without returning
  three values, and a loop body cannot change the stack.
- The 12-bit file takes 0.76 s against big12's 0.49 s. Its entropy data is
  3.7 times larger (6.8 MB against 1.8 MB: the same quality setting keeps
  four more bits of every coefficient), and its samples live in `array i32`
  planes and upsampling buffers (4 bytes a sample against 1) and leave as
  two big-endian bytes; the oracle's time rises in about the same
  proportion (0.14 s against 0.08 s).
- The YCCK file takes 0.70 s: 4.3 MB of entropy data at quality 85 with two
  full-size planes, then the YCbCr conversion plus three subtractions and
  three multiply-divides a pixel. The CMYK file, with a fourth plane and
  2.7 MB of entropy data, does only the multiply-divides (colour.ink).
