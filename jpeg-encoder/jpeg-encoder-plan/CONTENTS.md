# Whackford JPEG encoder: plan bundle

Planning documents for the JPEG encoder, the second half of the image
sanitiser. The decoder (`../../jpeg-decoder/`) turns hostile input into
pixels or a refusal; the encoder turns those pixels into a fresh, fully
conformant baseline JPEG. A sanitised file is always a fresh encode, so any
quirk the decoder tolerates in its input never reaches the output.

## Start here

1. **ARCHITECTURE.md**: purpose, the output profile, the two programs
   (`encode`, `clean`), module layout, data flow, the libjpeg behaviours to
   mirror.
2. **MILESTONES.md**: E0 to E7, each with its byte-exact gate.
3. **SECURITY.md**: what reaches the output, the metadata rules, the
   refusal codes.
4. **TESTING.md**: the cjpeg oracle, the harness, the inputs, the grid.

## Conventions

- Whackford is the implementation language; tooling is Python.
- Metric units; file sizes in MB (10^6 bytes) unless stated.
- Contract first: `declare` and `test`, then the body; `wack unresolved`
  is the to-do list.
- Anything Whackford makes awkward goes in `jpeg-encoder/docs/language-feedback.md`.
  Compiler and prelude changes stay general, never JPEG-specific.
- Correct first: speed passes come after E7, separately, and stay
  byte-exact.
