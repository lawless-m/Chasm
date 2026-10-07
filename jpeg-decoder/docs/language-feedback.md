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
