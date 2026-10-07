# Security model

## Threat model

The input is attacker-controlled. The attacker's goals, in order of what we
must prevent:

1. Code execution or memory disclosure on the host
2. Denial of service: hang, unbounded memory, unbounded CPU
3. A decode that silently produces wrong output (less serious, but a
   conversion service should not launder a broken file into a valid one
   that differs from what every other decoder shows)

We do not care about: metadata preservation (deliberately dropped), exotic
modes (refused), or recovering damaged photos (strict mode refuses).

## Layers

1. **Language.** Whackford's checked `bytes`, `array`, `vec`, `map` and
   WasmGC structs. The decoder has no `raw` words, so there is no address
   arithmetic anywhere in it. Audit: `wack words jpeg/*.wack | grep raw`
   must list nothing of ours.
2. **Wasm.** Linear memory is isolated by construction; a trap stops the
   instance.
3. **Runtime.** wasmtime with a preopened directory containing only the
   request's files, a maximum memory, and fuel or a wall-clock timeout.
   Confirm the exact flags with `wasmtime run --help` for the installed
   version; the intent is: `--dir=<reqdir>` only, no network, no env,
   memory capped at the budget plus headroom, CPU capped by fuel, and an
   outer `timeout` as the last resort.
4. **Process.** One process per request; memory is reclaimed by exit. The
   front end never parses the image itself.

## Invariants the decoder must hold

- **Bounded work.** Every loop is bounded by header-declared geometry
  (MCUs, blocks, components, scans) or by the input length, never by "until
  the data says stop" alone. Scan count is capped (`MAX_SCANS`, default
  256; a progressive image legitimately needs a few dozen at most).
- **Hard end of data.** The bit reader has an exhausted state. Reading past
  the end yields the flag; the scan layer refuses (strict) or finishes the
  current block with zeros exactly once and stops (lenient). It never spins.
- **Allocate once, after the budget.** No allocation after the frame header
  has been checked. Memory needed is a function of the header, computed and
  compared with `MEM_BUDGET` before `bytes.new`.
- **Refuse at detection.** Malformed input produces a refusal code at the
  point the inconsistency is first visible, not a bounds trap later. A trap
  in the fuzzer is a bug.
- **No `f64` in the decode path.** Deterministic integer results.
- **Nothing from the input reaches the output except samples.** No segment
  bytes are ever copied to the output.

## Limits (all in `limits.wack`, all configurable)

| Constant          | Default      | Why                                        |
| ----------------- | ------------ | ------------------------------------------ |
| `MAX_INPUT_BYTES` | 64 MB        | a JPEG larger than this is not a photo     |
| `MAX_DIM`         | 16384        | per side                                   |
| `MAX_PIXELS`      | 80 MP        | the published service limit                |
| `MEM_BUDGET`      | 1 GB         | self-hosted; 90 MB for a Worker            |
| `MAX_COMPONENTS`  | 4            | the spec allows 255; nothing uses > 4      |
| `MAX_SCANS`       | 256          | bounds progressive work                    |
| `MAX_SEGMENT`     | 65535        | the spec's own limit                       |
| `MAX_TABLES`      | 4 each       | the spec's own limit                       |

Memory needed = input + coefficient store (progressive) + one MCU row of
component planes + one output row + fixed tables, each rounded up to whole
MCUs. The store is the dominant term: 2 bytes x 64 x blocks, where blocks is
summed over components after sampling. A 4:2:0 image has 1.5 x (pixels/64)
blocks; a 4:4:4 one has 3 x; CMYK 4:4:4 has 4 x.

## Refusal codes

Stable identifiers, one per class of problem, on stderr as
`REFUSED <CODE>: <message>` with exit status 1.

| Code                  | Meaning                                                   |
| --------------------- | --------------------------------------------------------- |
| `NOT_JPEG`            | no SOI, or garbage before it                              |
| `TRUNCATED`           | file or segment ends early                                |
| `BAD_SEGMENT_LENGTH`  | length inconsistent with content                          |
| `UNSUPPORTED_SOF`     | lossless, hierarchical, differential, unknown             |
| `BAD_PRECISION`       | not 8 or 12                                               |
| `LIMIT_DIM`           | width or height over `MAX_DIM`                            |
| `LIMIT_PIXELS`        | over `MAX_PIXELS`                                         |
| `LIMIT_MEMORY`        | computed need over `MEM_BUDGET`                           |
| `LIMIT_INPUT`         | input over `MAX_INPUT_BYTES`                              |
| `BAD_COMPONENT`       | count, duplicate id, illegal sampling factor              |
| `BAD_TABLE`           | DQT/DHT/DAC malformed, over-subscribed Huffman, bad index |
| `MISSING_TABLE`       | scan references an undefined table                        |
| `BAD_SCAN_HEADER`     | component not in frame, Ss/Se/Ah/Al out of range          |
| `BAD_PROGRESSION`     | overlapping or out-of-order progressive scans             |
| `BAD_RESTART`         | RST out of sequence, or interval mismatch                 |
| `BAD_ENTROPY_DATA`    | invalid Huffman code, impossible run, bad arithmetic byte |
| `LIMIT_SCANS`         | over `MAX_SCANS`                                          |
| `NO_FRAME`            | SOS before SOF, or EOI with no scan                       |
| `TRAILING_GARBAGE`    | strict mode only: bytes after EOI beyond the allow-list   |

Exit 2 with `TRAP: <message>` is a wasm trap: a decoder bug, logged for
fixing, treated by the front end as a refusal.

## Strict and lenient

Strict is the default for the service. Lenient exists for development and
for comparing behaviour with libjpeg on damaged files.

Allowed in strict mode (benign, common in the wild):

- Bytes after EOI (ignored, not copied)
- Missing EOI when the last scan has consumed exactly the expected MCUs
- Fill bytes (FF FF ...) before a marker
- Padding bits in the last byte of entropy data being non-1 (libjpeg warns)
- APPn/COM segments of any content (skipped by length)
- A DHT or DQT redefined between scans (legal)

Lenient additionally:

- Truncated entropy data: finish with zeros, output what was decoded
- RST out of sequence: resynchronise as libjpeg does
- Bad Huffman code: treat as EOB for the block, as libjpeg does
- Decode a partial progressive image

Lenient mode is never exposed by the service.

## Testing the boundary

See TESTING.md: fuzz corpora from day one, a mutation fuzzer, per-file time
and memory caps in the harness, and a regression corpus of every file that
ever trapped or hung.
