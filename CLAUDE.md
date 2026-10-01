# Working on Chasm

Rust workspace. Read `docs/reference.md` before writing `.chasm` code;
`ARCHITECTURE.md` and `LANGUAGE.md` hold the design, and each ends with a
"Decisions taken" section. Record new decisions there.

## Layout

- `crates/core`: lexer, parser, checker and emitter (`check.rs`), module assembly (`module.rs`), driver (`program.rs`), memory layout (`layout.rs`), primitives (`prims.rs`), prelude written in Chasm (`prelude.chasm`). **No I/O**: it must keep building for `wasm32-unknown-unknown`.
- `crates/runtime`: ring servicing (`lib.rs`), native namespace (`namespace.rs`), wasmtime runner and test runner (`native.rs`).
- `crates/cli`: the `chasm` binary. Every command builds a JSON report; text is rendered from it.
- `examples/*.chasm`: every example must check and its tests must pass (enforced by `crates/cli/tests/examples.rs`).

## Before pushing

```
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo build -p chasm-core --target wasm32-unknown-unknown
```

## Conventions

- Diagnostic codes (`core/src/diag.rs`) are a contract: add new ones, don't rename.
- Every word has a declared effect; the checker is the single source of truth.
- Contract first: `declare` + `test`, then the body. `chasm unresolved` is the to-do list.
- Metric units in docs and messages.
