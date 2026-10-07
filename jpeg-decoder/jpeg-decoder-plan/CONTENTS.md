# Whackford JPEG decoder: plan bundle

Planning documents for a JPEG decoder written in Whackford
(https://github.com/lawless-m/Whackford), the core of a self-hosted safe
image conversion sandbox. No code yet: these documents are the brief for
Claude Code.

## Start here

1. **ARCHITECTURE.md**: purpose, scope, deployment (wasmtime process per
   request behind a small C# or Rust front end), module layout, data flow,
   the key interfaces, memory budget, how to match libjpeg-turbo exactly,
   decisions. Read this first.
2. **MILESTONES.md**: M0 to M9, each as a set of declared words with tests
   first and a corpus exit criterion. The order of work.
3. **SECURITY.md**: threat model, the invariants the decoder must hold,
   limits and their defaults, the refusal codes, strict versus lenient.
   Governs M0 and is re-checked at every milestone.
4. **TESTING.md**: unit tests per module, the corpus layout and sources, the
   djpeg oracle and the CMYK exception, the harness, the mutation fuzzer, CI.
5. **JPEG-NOTES.md**: the format crib for the implementer, with the libjpeg
   behaviours to mirror and the places decoders usually go wrong.

## Conventions

- Whackford is the implementation language for the decoder. Tooling
  (harness, fuzzer, corpus generation) is Python. The service front end is
  C# or Rust.
- Metric units throughout; file sizes in MB (10^6 bytes) unless stated.
- Keep `docs/language-feedback.md` in the project for anything Whackford
  made awkward: the decoder is also a stress test of the language.

## First actions for Claude Code

1. Read the Whackford reference (`docs/reference.md` in the language repo)
   and `examples/bytes.wack`, `examples/files.wack`.
2. Create the project layout from ARCHITECTURE.md with `declare` stubs and
   the M0 tests.
3. Obtain the libjpeg-turbo testimages into `corpus/primary/` and pin a
   djpeg version.
4. Write `tools/harness.py` before any decoding code.
