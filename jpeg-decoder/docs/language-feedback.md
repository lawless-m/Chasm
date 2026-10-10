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
  helper (`hex` in `jpeg-shared/fixtures.wack`).

## Files

- There is no way to learn a file's size before reading it, so the loader
  (`source.load`) starts with a 1 MiB buffer and doubles it whenever the
  file fills it, up to `MAX_INPUT_BYTES` + 1, copying what it has read so
  far each time (`source.grow`).

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
- `wack run` and `wack test` cache both compiles under `~/.cache/wack`: the
  Whackford compile's output in `compile/`, keyed by the sources, the
  prelude, the options and the compiler build, and wasmtime's machine code
  in `wasmtime/` (ARCHITECTURE.md section 28 item 4). A corpus run pays
  neither for an unchanged decoder: a decode of a 1x1 file takes 0.009 s,
  and the harness over the 73 files of `corpus/synthetic-m2` 0.23 s.
  `check` and `build` always compile; there is no native runner for a built
  module without wasmtime.
- `wack run` does not copy the linear memory when main returns; `wack test`
  reads it back to render `str` and `bytes` results. A decode's peak RSS is
  therefore the program's own allocations plus the host's few megabytes.

## Speed

Measured with `tmp/p5/time.py` and the release compiler
(`../target/release/wack run`, wasmtime with its function inliner on,
bounds checks on, both compiles cached). Each row's `wack run` is the wall
median of three decodes to a real file and its CPU the minimum child CPU
time of those three (user plus system). Speed is judged on CPU, because
wall time includes write-back waits that depend on the run sequence and on
background load: in P2 a row read 0.67 s wall in every pass of the timing
script while an alternating A/B showed the same file's decode faster by
CPU and 0.44 s wall on its own. The table is the per-row minimum of two
passes (`tmp/p5/final.txt`). The files were made with the oracle's cjpeg
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
The decode rate is 12 megapixels over the run less the startup, 0.008 s.
The oracle writes to `/dev/null`; the decoder writes a real file, which
costs it about 0.1 s of wall time over `/dev/null` on these files (big12
fancy 0.38 s against 0.24 s) but only about 0.03 s of CPU.

| File | Mode | `wack run` | CPU | `wack run --opt` | Decode rate | Oracle |
|---|---|---|---|---|---|---|
| big12, baseline | fancy (default) | 0.35 s | 0.232 s | 0.65 s | 34.9 megapixels/s | 0.08 s |
| big12, baseline | plain (`nosmooth`) | 0.30 s | 0.191 s | 0.60 s | 40.7 megapixels/s | 0.06 s |
| big12p, progressive | fancy (default) | 0.50 s | 0.380 s | 0.80 s | 24.3 megapixels/s | 0.13 s |
| big12p, progressive | plain (`nosmooth`) | 0.45 s | 0.338 s | 0.77 s | 26.9 megapixels/s | 0.11 s |
| big12s3, three scans | fancy (default) | 0.40 s | 0.292 s | 0.74 s | 30.8 megapixels/s | 0.08 s |
| big12s3, three scans | plain (`nosmooth`) | 0.36 s | 0.247 s | 0.66 s | 34.1 megapixels/s | 0.07 s |
| big12a, arithmetic | fancy (default) | 0.59 s | 0.484 s | 0.97 s | 20.5 megapixels/s | 0.22 s |
| big12a, arithmetic | plain (`nosmooth`) | 0.56 s | 0.445 s | 0.89 s | 21.9 megapixels/s | 0.21 s |
| big12pa, progressive arithmetic | fancy (default) | 0.65 s | 0.525 s | 0.96 s | 18.8 megapixels/s | 0.26 s |
| big12pa, progressive arithmetic | plain (`nosmooth`) | 0.59 s | 0.487 s | 0.94 s | 20.7 megapixels/s | 0.24 s |
| big12b12, 12-bit | fancy (default) | 0.62 s | 0.361 s | 0.86 s | 19.7 megapixels/s | 0.14 s |
| big12b12, 12-bit | plain (`nosmooth`) | 0.59 s | 0.358 s | 0.91 s | 20.6 megapixels/s | 0.12 s |
| big12cmyk, CMYK | fancy (default) | 0.39 s | 0.275 s | 0.72 s | 31.3 megapixels/s | 0.12 s |
| big12cmyk, CMYK | plain (`nosmooth`) | 0.41 s | 0.273 s | 0.63 s | 30.3 megapixels/s | 0.10 s |
| big12ycck, YCCK | fancy (default) | 0.54 s | 0.390 s | 0.77 s | 22.6 megapixels/s | 0.16 s |
| big12ycck, YCCK | plain (`nosmooth`) | 0.52 s | 0.392 s | 0.77 s | 23.6 megapixels/s | 0.15 s |

- The oracle is libjpeg-turbo 3.2.0 in plain C without SIMD. The decoder
  runs 3 to 6 times slower than it by wall time to a real file, and 2 to 4
  times writing to `/dev/null` as the oracle does; closest on arithmetic
  files, where the oracle itself is three times slower than on Huffman ones.
- The Whackford compiler inlines small words itself (ARCHITECTURE.md
  section 28 item 5): a word of at most `INLINE_LIMIT` (256) instructions,
  defined once and earlier in the program, is spliced into its callers, so
  struct field accessors and small helper words are no longer calls in the
  wasm. Cranelift's own inliner (item 3) handles what remains. Against the
  compiler without it, every row's CPU falls, progressive files most
  (big12p fancy 0.524 s to 0.380 s). `wack run --opt` (Binaryen -O3) is
  slower than plain `wack run` on every row: its inlining buys nothing more,
  and its own time is paid on each run.
- Startup is 0.008 s: with both compiles cached, a run hashes the sources
  and the prelude, reads one entry, loads the cached module and reads the
  input (a one-line program runs in 0.005 s). Uncached, the Whackford
  compile is about 0.035 s of it (`wack build --no-opt` of the decoder
  takes 0.039 s). The loader's input buffer starts at 1 MiB and doubles as
  the file comes in (`source.load`), so a small file touches 18 MB of
  memory.
- In a baseline decode (big12 fancy, 0.35 s best of seven, one stage
  removed at a time in scratch copies, `tmp/p3/stages.py`) the stages take:
  row writes 34%, IDCT 26%, entropy decoding and the scan loop 17%, colour
  conversion 11%, upsampling 9%, startup 3%. The row writes go out in 1 MiB
  batches (`ppm.row` collects rows in the shared `out` sink, `out.close` flushes), 35 `host.write`
  calls for the 36 MB image, so their cost is the kernel taking 36 MB into
  the file, not the calls.
- In plain mode, a YCbCr frame whose chroma is halved across (h2v1 or h2v2)
  is converted by `colour.row-h2` straight from the chroma planes, as
  libjpeg's merged upsampler does: each chroma sample's three table terms
  are looked up once for its pair of pixels, and the chroma planes are not
  replicated first. big12 nosmooth 0.259 s to 0.227 s of CPU (best of
  seven, alternating runs); fancy mode does not take this path.
- A greyscale row is one `bytes.put` copy from the upsampled plane: a
  12-megapixel greyscale file (`tmp/p3/big12g.jpg`) 0.128 s to 0.121 s of
  CPU.
- Tried and not kept: `colour.put12` writing each 12-bit sample with one
  `bytes.u16-at!` of the byte-swapped value instead of two `bytes.at!`. As
  a prelude word it was slower (big12b12 fancy 0.400 s to 0.412 s of CPU);
  as an inline primitive it is level (0.407 s both), so the two byte stores
  stay. Also reverted in P2: `bits.fill` over locals with one write-back
  (inside noise). A branchless clamp with `select` is 2 to 4% slower on a
  photograph and 3 to 5% faster on YCCK, so `colour.clamp` keeps its
  branches.
- `upsample.plain` copies a full-width row (hexp 1) with one `bytes.put`
  and replicates a subsampled one by reading each source sample once and
  storing it hexp times; `upsample.h2v2-row` computes each column sum once
  and slides it through last, this and next, as libjpeg's
  h2v2_fancy_upsample; the 12-bit words do the same, `upsample.plain12`
  copying a full-width row with one `array.copy` (big12b12 0.406 s to
  0.401 s of CPU). `idct.block` and
  `idct.block12` load each column's and row's eight values once, shared by
  the zero-AC test and the transform. `huffman.decode`, holding 8 bits,
  takes the lookahead and drops the code's bits straight from buf and
  nbits, and `bits.get` checks for enough bits once. `arith.decode` keeps
  c, a and ct in locals for a decision; across words the registers still
  live in the arith record, as a word cannot hand three values back to a
  loop that keeps them and a loop body cannot change the stack.
- The store path costs about 0.05 s of CPU on 12 megapixels for the same
  entropy work (big12s3 against big12): every coefficient goes into the
  store and back out through `bytes.u16-at!` and `bytes.u16-at`, and the
  IDCT runs in a second pass. Ten progressive scans cost another 0.20 s
  (big12p against big12s3), mostly entropy decoding of the refinement
  scans.
- Arithmetic decoding costs about 0.19 s of CPU more than Huffman decoding
  of the same picture through the same store (big12a against big12s3). A
  block takes about 52 binary decisions for 7 nonzero AC coefficients
  (counted on `primary/testimgari.jpg`); each decision fetches and stores
  its statistics byte with `bytes.at` and `bytes.at!` and indexes the state
  table, every access bounds-checked.
- The 12-bit file takes 0.40 s of CPU against big12's 0.26 s. Its entropy
  data is 3.7 times larger (6.8 MB against 1.8 MB: the same quality setting
  keeps four more bits of every coefficient), and its samples live in
  `array i32` planes and upsampling buffers (4 bytes a sample against 1)
  and leave as two big-endian bytes; the oracle's time rises in about the
  same proportion (0.13 s against 0.08 s).
- The YCCK file takes 0.45 s of CPU: 4.3 MB of entropy data at quality 85
  with two full-size planes, then the YCbCr conversion plus three
  subtractions and three multiply-divides a pixel. The CMYK file, with a
  fourth plane and 2.7 MB of entropy data, does only the multiply-divides
  (colour.ink) and takes 0.31 s.

## The IDCT against a hand-emitted built-in

A built-in can only beat a word by doing what the word cannot express or what
the compiler does badly: both become wasm, and both go through the same
Cranelift. So the difference between the decoder's `idct.block` and a
hand-emitted equivalent measures general compiler gaps. The experiment lives on
the local branch `p4-idct-builtin`, which is never merged: `idct.islow8`, a
primitive computing exactly what `idct.block` computes, built in five stages
that each remove one cost. Each stage passes the decoder's 1556 tests and is
byte-exact on the 228 IDCT-heavy files (synthetic-m2, synthetic-m4 and
m2-decoded) in both modes, and is timed by interleaved child-CPU A/B (nine pairs
after a warm-up, minimum) on big12, big12s3 and big12b12. The wasm is counted
from Binaryen's stack IR and the machine code from objdump of wasmtime's
compiled module (`tmp/p4/`). big12b12 is a 12-bit file whose IDCT is
`idct.block12`, untouched, so its column is a control.

| Stage | Removes | big12 CPU | big12s3 CPU | big12b12 CPU | idct.block wasm | idct.block machine | Byte-exact |
|---|---|---|---|---|---|---|---|
| S0 | the word as on main | 0.268 s | 0.319 s | 0.399 s | 1989 | 1039 | P3 gate: 1689/1689 both modes |
| S1 | nothing: transliteration, same checks, helpers called | 0.261 s | 0.311 s | 0.398 s | 2007 | 1033 | 228/228 both modes |
| S2 | the calls to i64, dct.descale and idct.1d (helpers inline) | 0.232 s | 0.288 s | 0.403 s | 2328 | 1015 | 228/228 both modes |
| S3 | the per-access bounds checks (hoisted to entry) | 0.223 s | 0.283 s | 0.402 s | 1611 | 560 | 228/228 both modes |
| S4 | address arithmetic and local traffic (direct addressing) | 0.215 s | 0.278 s | 0.399 s | 1009 | 467 | 228/228 both modes |
| S5 | i64 where i32 is exact on in-range data (upper bound) | 0.215 s | 0.274 s | 0.399 s | 975 | 463 | 228/228 both modes; fuzz: clean (646/646 both modes) |

The word's IDCT is about 0.070 s of big12's 0.268 s of CPU (26%, from the stage
breakdown above). Against that:

- **Calls the engine's inliner leaves in a large word: 0.030 s, 43% of the
  IDCT** (S2 minus S1). Cranelift's inliner alone leaves 8 of the 17 calls of
  `i64` (a one-instruction word), 10 of the 17 descales and one of the two
  butterflies as calls inside `idct.block`. The Whackford compiler now
  inlines words of at most `INLINE_LIMIT` (256) instructions at their call
  sites (ARCHITECTURE.md section 28 item 5), so `idct.block` has none of
  those calls in its wasm (`tmp/p5/s2-wasm.md`) or its machine code
  (`tmp/p5/s2-mc.md`). With the word's source unchanged, big12 fancy goes
  from 0.266 s to 0.232 s of CPU (`tmp/p5/baseline.txt`, `tmp/p5/final.txt`),
  the same as stage S2's 0.232 s, and every other program's small words gain
  the same way.
- **Per-access bounds checks: 0.009 s, 13%, still open** (S3 minus S2). The 52 checks halve
  the machine code (1015 to 560 instructions), but they never fire and so are
  predicted perfectly: the cost is their instructions, the lengths kept live
  and the spills those force. The general fix is bounds-check elimination over
  `times` loops of known range, or a fixed-size array type whose length the
  checker knows, keeping Whackford's safety contract. The hoisted form traps
  before any store on an invalid call, rather than at the first bad access.
- **Address arithmetic and local traffic: 0.003 s, 4%** (S4 minus S3), inside
  the noise. The wasm shrinks by 37%, but Cranelift already folds constant
  index arithmetic into addressing modes and allocates the temporaries to
  registers. Folding constant offsets into memarg offsets is a code-size
  improvement, not a speed one.
- **i64 where i32 is exact: 0.005 s, 7%** (S5 minus S4), an upper bound. On
  x86-64 a 64-bit multiply costs what a 32-bit one does. i32 is byte-exact on
  the 228 files and on the whole fuzz corpus (646 files, both modes), but
  libjpeg-turbo computes in 64 bits, and only a proof for all inputs, which
  the compiler has no range analysis to give, would make it safe under the
  decoder's hostile-input contract. Not worth pursuing.
- **The plumbing: nil** (S1 minus S0, −0.007 s, in the noise). A primitive
  emitted through the compiler's own handlers is the word.

The best stage takes the IDCT from about 0.070 s to about 0.016 s, a 77% cut,
and the decode from 0.268 s to 0.215 s of CPU. Nearly all of that comes from
two general compiler changes: inlining small words and eliminating provable
bounds checks. What no stage reaches is the vector form. libjpeg-turbo's
`jsimd` islow IDCT works on eight lanes at once, and that is not expressible in
Whackford at all, which has no `v128` type.
