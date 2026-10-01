# Chasm: Contents

Chasm (Chuck-Wasm) is a typed, concatenative, Forth-and-Factor-flavoured language that compiles to WebAssembly, with an interactive REPL. Implementation language: Rust. Licence: MIT.

## Where to start

1. **README.md**: what it is and how to run it.
2. **docs/reference.md**: the working language reference, i.e. what the compiler accepts today.
3. **ARCHITECTURE.md**: goals, crate layout, pipeline, runtime, milestones M0 to M6, and (section 13) the decisions taken while building M0 and M1.
4. **LANGUAGE.md**: the v1 language spec; section 12 records the M1 decisions.
5. **FUTURE.md**: explicitly *not* v1. Read it so v1 decisions do not close these off, but build none of it.
6. **CLAUDE.md**: how to work in the repository.
7. **examples/**: worked programs with tests.

## Working conventions

- Every word has a declared effect; the checker is the single source of truth.
- Contract first: `declare` a word's effect and tests, then fill in the body. `chasm unresolved` is the to-do list.
- All CLI commands produce text and JSON; text is rendered from the JSON.
- Metric units in docs and messages. Rust for everything; `wasm-encoder` and `wasmparser` for wasm, `wasmtime` for the native host.
- Open questions are listed at the end of ARCHITECTURE.md and LANGUAGE.md. Decide them in the smallest way that unblocks the current milestone and record the decision in the doc.
