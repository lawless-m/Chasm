# Working on Chasm

Rust workspace. Read `docs/reference.md` before writing `.chasm` code;
`ARCHITECTURE.md` and `LANGUAGE.md` hold the design, and each ends with a
"Decisions taken" section. Record new decisions there.

## Layout

- `crates/core`: lexer, parser (top-level `:`, `declare`, `test` and `struct` items), checker and emitter (`check.rs`), module assembly (`module.rs`), driver (`program.rs`), memory layout (`layout.rs`), primitives (`prims.rs`), prelude written in Chasm (`prelude.chasm`). **No I/O**: it must keep building for `wasm32-unknown-unknown`.
- `crates/runtime`: ring servicing (`lib.rs`), native namespace (`namespace.rs`), wasmtime runner and test runner (`native.rs`), native REPL host (`repl.rs`).
- `crates/web`: `chasm-web`, the C-ABI cdylib the browser loads (the REPL session; no I/O, builds for `wasm32-unknown-unknown`).
- `web/`: the static browser REPL (`web/build.sh`, `web/serve.py`, node checks under `web/test/`).
- `crates/cli`: the `chasm` binary. Every command builds a JSON report; text is rendered from it.
- `examples/*.chasm`: every example must check and its tests must pass (enforced by `crates/cli/tests/examples.rs`).
- `bench/`: run-speed benchmarks. `bench/<task>.chasm` replaces the `main` of `examples/<task>.chasm`; `bench/rust/` and `bench/js/` are ports. `python3 bench/run.py --record` appends results to `docs/performance.md`.

## Before pushing

```
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test                                    # needs Binaryen's wasm-opt 121+ on the path
cargo build -p chasm-core --target wasm32-unknown-unknown
sh web/build.sh
node web/test/compiler.mjs && node web/test/ring.mjs && node web/test/node-repl.mjs
node web/test/node-structs.mjs              # needs node 22 (CI); locally:
sh web/test/headless.sh test/structs.html STRUCTS   # the same scenario in headless Vivaldi
```

## Conventions

- Diagnostic codes (`core/src/diag.rs`) are a contract: add new ones, don't rename.
- Every word has a declared effect; the checker is the single source of truth.
- Contract first: `declare` + `test`, then the body. `chasm unresolved` is the to-do list.
- Metric units in docs and messages.
