# Milestones

Each milestone is a set of `declare`d words with `test` lines written first,
so `wack unresolved` is the to-do list and `wack test` is the progress bar.
A milestone is done when its tests pass, its corpus subset passes the
harness (TESTING.md), and no `raw` word has appeared (`wack words`).

Keep `docs/language-feedback.md` as you go: anything in Whackford that was
awkward, slow or missing.

## M0: skeleton and the security boundary

- `limits.wack`, `refuse.wack`: codes, budget constants, the refusal word,
  `main` distinguishing refusal exit 1 from trap exit 2.
- `source.wack`: load `/file/in.jpg` into one `bytes` buffer; refuse over
  `MAX_INPUT_BYTES`.
- `markers.wack`: walk segments by length; refuse bad lengths, truncation,
  unknown or out-of-scope SOF markers, missing SOI. Parse DQT (8 and 16 bit
  tables), DHT (counts and symbols; validate counts), DRI, SOF0/1/2/9/10
  headers, APP0 JFIF and APP14 Adobe (transform flag), SOS header.
- `frame.wack`: component geometry, MCU dimensions, blocks per component,
  budget calculation. Refuse illegal sampling factors, duplicate component
  ids, table references to undefined tables, dimensions and pixel count over
  limits, zero dimensions (unless DNL follows).
- Harness: `tools/harness.py` runs the decoder over a directory with a
  timeout and reports decoded / refused / trapped / hung per file.
- Exit: every file in the fuzz corpora is refused or reaches "SOS, not yet
  implemented" within the timeout, and nothing traps.

## M1: Huffman and the bit reader

- `bits.wack`: byte-stuffing (FF 00), marker detection (FF xx leaves the
  marker for the scan layer and enters end-of-data state), RST handling
  hook, hard end-of-data: once the data is exhausted the reader returns a
  flagged state, never zeros forever.
- `huffman.wack`: canonical code construction from DHT (Annex C), a 9-bit
  lookup table plus the maxcode/valptr slow path (Annex F.2.2.3). Refuse
  tables whose code lengths over-subscribe (Kraft sum > 1) or that leave
  gaps libjpeg would reject.
- Tests: known DHT segments to known code tables; known bitstrings to known
  symbols; stuffing and marker edge cases; truncated data yields the
  end-of-data flag.

## M2: baseline sequential, greyscale to colour

- `scan.wack`: sequential scan decode per MCU, DC prediction per component,
  AC run/size decode, EOB and ZRL, restart intervals (reset predictors,
  resynchronise on RSTn, refuse a mismatched RST number in strict mode).
- `idct.wack`: islow with dequantisation, 8-bit, range-limit table.
- `rows.wack`, `ppm.wack`: MCU-row driver, row sink, P5 writer, then P6.
- `upsample.wack`: plain replication for any sampling factor.
- `colour.wack`: greyscale, YCbCr (fixed-point tables), RGB (no transform).
- Order: greyscale baseline -> 4:4:4 -> 4:2:0 -> 4:2:2 -> odd factors ->
  restart markers -> DNL.
- Tests: a known coefficient block through islow to known samples (take
  expected values from a libjpeg run); zigzag; DC prediction across a
  restart.
- Exit: byte-exact against `djpeg -dct int -nosmooth -pnm` on every baseline
  file in the primary corpus.

## M3: fancy upsampling

- h2v1 and h2v2 triangle filters exactly as jdsample.c (including the edge
  handling and the 3/4-1/4 rounding pattern). Other factors stay plain,
  as in libjpeg.
- Exit: byte-exact against default `djpeg -dct int -pnm` on the same files.
  The harness picks the oracle flags per test case.

## M4: progressive

- `coeffs.wack`: 16-bit packed store, allocated once after the budget check.
- `progressive.wack`: DC first (Al shift), DC refine (one bit per block),
  AC first with EOB runs (EOBn), AC refine (correction bits, the
  non-obvious ordering from G.1.2.3), spectral selection and successive
  approximation validation per component (refuse overlapping or
  out-of-order refinement, Ah/Al inconsistencies, AC scans with more than
  one component).
- After the last scan (or EOI), drive `rows` from the store.
- Exit: byte-exact on the progressive corpus files; a partial progressive
  file (truncated mid-scan) is refused in strict mode, or decodes what it
  has if the lenient flag is set.

## M5: arithmetic coding

- `arith.wack`: the QM decoder of Annex D: Qe table, probability estimation
  state machine, conditional exchange, renormalisation, byte-in with
  stuffing and marker handling, the statistics bins for DC (per
  conditioning category, DAC L/U parameters) and AC (Kx).
- `entropy.wack` dispatch so `scan` and `progressive` are unchanged.
- Exit: byte-exact on the arithmetic corpus files (libjpeg-turbo's
  `testimgari.jpg` and progressive-arithmetic variants).

## M6: 12-bit

- Precision carried as a frame parameter: IDCT scaling (islow's 12-bit
  constants), range-limit table size, colour table scaling, 16-bit PPM
  samples, DC/AC magnitude categories up to 15/16.
- Exit: byte-exact against 12-bit djpeg output (maxval 4095).

## M7: colour spaces and Adobe

- Decision tree (JPEG-NOTES.md): 1 component greyscale; 3 components JFIF or
  no marker -> YCbCr, Adobe transform 0 -> RGB, transform 1 -> YCbCr,
  component ids 'R','G','B' -> RGB; 4 components Adobe transform 2 -> YCCK
  else CMYK; Adobe files have inverted CMYK (Photoshop convention), applied
  as libjpeg-turbo does.
- YCCK -> CMYK, CMYK -> RGB for the PPM, raw plane dump for the harness.
- Exit: raw planes byte-exact against Pillow; PPM visually correct in GIMP.

## M8: robustness

- The mutation fuzzer (TESTING.md) over the whole corpus, long runs.
- Every trap found becomes a refusal with a code and a regression test
  file in `corpus/regressions/`.
- Timing: a per-file CPU budget in the harness; any file over it is a bug
  (unbounded work).
- Memory: the harness runs wasmtime with a max memory; any failure is a
  bug in the budget calculation.
- Exit: a week of continuous fuzzing with zero traps, zero hangs, zero
  memory failures.

## M9: service

- `wack build --wasi`; the front end (C# or Rust) that spawns wasmtime per
  request with limits, maps exit codes, returns PPM or the refusal.
- Operational limits documented; the pixel budget published as the service
  limit.
- Later: the encoder project, consuming `rows`.

## Cross-cutting from the start

- No `raw` words. Check at every milestone.
- No allocation in loops. Review at every milestone.
- `wack fmt --check` and `wack test` in CI (the Whackford repo's workflow
  shows the pattern).
