# Working on Whackford

Rust workspace. Read `docs/reference.md` before writing `.wack` code;
`ARCHITECTURE.md` and `LANGUAGE.md` hold the design, and each ends with a
"Decisions taken" section. Record new decisions there.

## Layout

- `crates/core`: lexer, parser (top-level `:`, `declare`, `test`, `struct` and `union` items), checker and emitter (`check.rs`, with the unwind/rewind transform for processes, `Unwind`), module assembly (`module.rs`), driver (`program.rs`), memory layout (`layout.rs`), primitives (`prims.rs`), prelude written in Whackford (`prelude.wack`). **No I/O**: it must keep building for `wasm32-unknown-unknown`.
- `crates/runtime`: ring servicing (`lib.rs`), native namespace (`namespace.rs`), HTTP client (`net.rs`), 9p client (`ninep.rs`), wasmtime runner, test runner and process driver (`native.rs`), native scheduler (`proc.rs`), native REPL host (`repl.rs`).
- `crates/web`: `wack-web`, the C-ABI cdylib the browser loads (the REPL session; no I/O, builds for `wasm32-unknown-unknown`).
- `web/`: the static browser REPL; its worker runs processes by the same unwind/rewind transform as the native hosts (`web/drive.js`) (`web/build.sh`, `web/serve.py`, node checks and headless pages under `web/test/`).
- `site/`: the documentation site generator (`build.py` writes `tmp/site/`, published to GitHub Pages by `.github/workflows/pages.yml`; `check.py` tests every ```wack block in `docs/reference.md`, `docs/tour/` and `site/index.md`; `headless.sh` opens the built REPL through a plain server). `docs/tour/`: the guided tour, one page per file in filename order.
- `crates/cli`: the `wack` binary. Every command builds a JSON report; text is rendered from it.
- `examples/*.wack`: every example must check and its tests must pass (enforced by `crates/cli/tests/examples.rs`).
- `bench/`: run-speed benchmarks. `bench/<task>.wack` replaces the `main` of `examples/<task>.wack`; `bench/rust/` and `bench/js/` are ports. `python3 bench/run.py --record` appends results to `docs/performance.md`.

## Before pushing

```
cargo fmt --all
cargo run -q -p wack-cli -- fmt examples/*.wack bench/*.wack
cargo clippy --all-targets -- -D warnings
cargo test                                    # needs Binaryen's wasm-opt 121+ on the path
cargo build -p wack-core --target wasm32-unknown-unknown
sh web/build.sh
node web/test/compiler.mjs && node web/test/ring.mjs && node web/test/node-repl.mjs && node web/test/net.mjs && node web/test/proc.mjs
python3 site/check.py
python3 site/build.py
sh site/headless.sh                           # the built REPL in headless Vivaldi: prints link ok
node web/test/node-structs.mjs              # the struct scenario; needs node 22 (CI)
node web/test/node-procs.mjs && node web/test/node-examples.mjs   # the process scenarios under node 22 (CI)
sh web/test/headless.sh test/structs.html STRUCTS   # the same scenarios in a real browser (headless Vivaldi)
sh web/test/headless.sh test/procs.html PROCS
sh web/test/headless.sh test/examples.html EXAMPLES   # examples/pipeline.wack and examples/alt.wack
```

## Conventions

- Diagnostic codes (`core/src/diag.rs`) are a contract: add new ones, don't rename.
- Every word has a declared effect; the checker is the single source of truth.
- Contract first: `declare` + `test`, then the body. `wack unresolved` is the to-do list.
- Metric units in docs and messages.
- Fenced code in the docs: ```wack is a complete program that is checked (by `site/check.py`) and gets a Try-it button; ```wack fragment and ```wack-repl are shown but not checked; other fences are not Whackford.
