# Testing

Two layers: Whackford `test` lines for units, and an external harness for
whole files against an oracle, with time and memory caps.

## Unit tests (Whackford `test` lines)

Write them before the word, against the `declare`. Expected values that need
computing (IDCT outputs, colour conversions) come from a one-off script that
runs the libjpeg algorithm in Python or from djpeg on a tiny synthetic file.

Per module, at least:

- `markers`: segment lengths, truncation, each refusal code reachable
- `bits`: stuffing, marker stop, end-of-data flag, bit ordering
- `huffman`: canonical construction from the standard tables in Annex K
  (K.3), decode of hand-assembled bitstrings, over-subscribed table refused
- `arith`: Annex D's own test sequence (the spec gives a 256-byte input and
  its expected decode; use it)
- `idct`: DC-only block, a single AC coefficient, a full block, 8 and 12 bit,
  all against islow outputs
- `upsample`: h2v1 and h2v2 edges and interior
- `colour`: the decision tree table (every combination of component count,
  JFIF, Adobe transform, component ids), and fixed-point conversions at
  the extremes and midpoint
- `progressive`: EOB run accounting, refinement bit ordering on a crafted
  block, scan validation refusals
- `coeffs`: packing round trip, negative values

Struct values cannot be test literals; test fields or derived scalars.

## Corpus

```
corpus/
  primary/        libjpeg-turbo testimages (baseline, progressive, arithmetic,
                  12-bit, greyscale, odd sampling); ITU T.83 conformance
                  streams where obtainable
  photos/         our own: phone photos (EXIF, large 4:2:0), scanner output,
                  Photoshop CMYK and YCCK exports, a few re-saved by GIMP
                  and ImageMagick (JFIF-less, RGB-tagged, odd factors)
  wild/           other decoders' regression images (Pillow, stb_image,
                  image-rs jpeg-decoder tests, Chromium's image corpus):
                  real-world oddities
  fuzz/           OSS-Fuzz seed corpora for libjpeg-turbo and others;
                  Google's old imagetestsuite broken JPEGs
  regressions/    every file that ever made our decoder trap or hang
  synthetic/      files we generate: every sampling factor combination,
                  every progressive scan script cjpeg accepts, DNL, DRI
                  with each interval, each refusal code
```

Each file in `primary`, `photos`, `wild` and `synthetic` gets a sidecar
`name.expect` saying `decode` with the oracle flags to use, or `refuse CODE`.
Fuzz files have no sidecar: the expectation is "refuse or decode, in time,
in memory, no trap".

Generate `synthetic/` with libjpeg-turbo's `cjpeg` (`-sample`, `-progressive`,
`-scans`, `-arithmetic`, `-precision 12`, `-restart`, `-grayscale`) from a
few PPM sources, plus a small script for the malformed ones (each refusal
code must have at least one file that triggers it).

## Oracle

libjpeg-turbo's `djpeg`, pinned to one version in the harness.

| Case                   | Oracle command                                    |
| ---------------------- | ------------------------------------------------- |
| 8-bit grey, RGB, YCbCr | `djpeg -dct int -nosmooth -pnm` (M2), `-dct int -pnm` (M3 on) |
| 12-bit                 | same; libjpeg-turbo 3.x writes maxval 4095        |
| CMYK, YCCK             | no djpeg PPM oracle; see below                    |
| refusals               | the sidecar's expected code; also record djpeg's exit status and message for comparison |

Comparison is byte-exact on the PPM. Any difference is a failure; there is no
tolerance mode (a tolerance would hide upsampling and rounding mistakes that
compound in progressive refinement).

**CMYK and YCCK.** djpeg refuses to write CMYK to PPM. The decoder's raw plane
dump (`out.raw`: width, height, components as a tiny header, then packed
samples component-interleaved) is compared byte-exact against Pillow opened
in CMYK mode and `tobytes()`'d; Pillow uses libjpeg-turbo and applies the
Adobe inversion the same way. Pin Pillow's version too. The PPM for CMYK
files is checked only by eye in GIMP during development.

## Harness

`tools/harness.py`:

- Runs the decoder per file (during development via `wack run` with
  `--mount corpus=... --mount out=...`; later via wasmtime on the WASI build,
  which also exercises the runtime limits).
- Per file: wall-clock timeout (default 20 s), memory cap (wasmtime's
  flag), captured stderr.
- Classifies: `pass` (matches oracle or expected refusal), `mismatch`
  (decoded, differs; writes a diff summary: first differing row and
  component, counts), `wrong-refusal` (refused with the wrong code or
  refused when it should decode), `trap`, `hang`, `oom`.
- `trap`, `hang` and `oom` always fail, including on fuzz files, and copy
  the file into `corpus/regressions/` with a note.
- Summary table per corpus directory; non-zero exit on any failure.
- `--filter` by milestone tag in the sidecar so early milestones run only
  what they should pass.

## Mutation fuzzer

`tools/mutate.py`, run against the whole non-fuzz corpus:

- Mutations: flip random bytes, set bytes to FF or 00, truncate at a random
  point, duplicate or delete a segment, swap two segments, corrupt a length
  field, splice the tail of one file onto the head of another, replace a
  DHT with an over-subscribed one, set huge dimensions in SOF.
- Only three outcomes are acceptable: decoded in time and memory, refused
  with a code, or (for the oracle comparison variant) the same outcome as
  djpeg in lenient mode. A trap, a hang or an OOM saves the input to
  `corpus/regressions/` and fails.
- Runs in CI for a fixed number of iterations per push; a long run before a
  release.

## CI

`wack fmt --check`, `wack test` on every module, the harness on `primary` and
`synthetic`, a short mutation run. The Whackford repo's GitHub workflow is the
template.

## Performance (not a gate, but tracked)

The harness records decode time per megapixel. Expect bounds checks to cost;
measure after `wack build` (Binaryen) and note hot spots in
`docs/language-feedback.md`. The 12 MP phone photo decode time is the
headline number for the service.
