# Chasm: Planning Documents

Chasm (Chuck-Wasm) is a typed, concatenative, Forth-and-Factor-flavoured language that compiles to WebAssembly, with an interactive REPL. Implementation language: Rust. Licence: MIT.

## Where to start

1. **ARCHITECTURE.md** — read first. Goals, crate layout, pipeline, runtime (memory layout, stacks, function table, I/O), the declare/define/redefine rules, dependency graph, export, REPL rules, machine-readable output, and milestones M0 to M6. Start building at **M0** and **M1**.
2. **LANGUAGE.md** — the v1 language spec: types, literals, numerics, strings, shuffles, locals, control flow, definitions, tests, I/O. The primitive set is "the wasm numeric instructions plus what is listed here".
3. **FUTURE.md** — explicitly *not* v1. Structs, arrays, functions as values, closures, WasmGC, namespaces. Read it so v1 decisions do not close these off, but build none of it.
4. **LICENSE** — MIT.

## Working conventions

- Every word has a declared effect; the checker is the single source of truth.
- Contract first: `declare` a word's effect and tests, then fill in the body. `chasm unresolved` is the to-do list.
- All CLI commands produce text and JSON; text is rendered from the JSON.
- Metric units in docs and messages. Rust for everything; `wasm-encoder` and `wasmparser` for wasm.
- Arrays and functions as values are v1; closures and structs are not. Open questions are listed at the end of ARCHITECTURE.md and LANGUAGE.md. Decide them in the smallest way that unblocks the current milestone and record the decision in the doc.
