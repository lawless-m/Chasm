# Whackford

Whackford (after Wackford Squeers; the command is `wack`) is a typed, concatenative language in
the Forth and Factor family that compiles to WebAssembly. Every word has a
stack effect, written or inferred, and the checker verifies each body against
it.

```
: square ( i32 -- i32 )  dup i32.mul ;
test square : 3 square -> 9

: main ( -- )  "7 squared is " print  7 square i32.to-str println ;
```

```
$ cargo run -p wack-cli -- run examples/basics.wack
$ cargo run -p wack-cli -- test examples/arrays.wack
$ cargo run -p wack-cli -- check --json examples/strings.wack
$ cargo run -p wack-cli -- repl
```

`wack repl` is interactive: type definitions and lines, see the stack after
each, redefine words live.

- https://lawless-m.github.io/Chasm/: the documentation site (guided tour, reference, word index and the browser REPL)
- `docs/reference.md`: how to write Whackford (start here)
- `docs/tour/`: the guided tour for newcomers
- `ARCHITECTURE.md`: goals, runtime, milestones, decisions
- `LANGUAGE.md`: the v1 language specification
- `FUTURE.md`: what is deliberately not v1
- `examples/`: worked examples, each with tests (`docs/examples.md`: the corpus and benchmark plan)
- `web/README.md`: the REPL in the browser
- `docs/performance.md`: run speed against Rust and JavaScript (`python3 bench/run.py`)
- `docs/editors.md`: `wack lsp` in Neovim, Helix and VS Code

Status: milestones M0 (skeleton), M1 (declared effects to wasm, CLI, native
host), M2 (interactive REPL, native and in the browser), M3 (dependency-graph
tooling: `deps`, `used-by`, `dead`, `)forget`), M4 (structs as WasmGC
structs) and M5 (whole-program export: dead words left out, unresolved words
refused, Binaryen) and M6 (polish and tooling: JSON output everywhere, the
`wack lsp` language server, `)force` in the REPL, `/net/http` with request
headers natively and in the browser, 9p mounts, `wack build --wasi`, and
examples for each) and M7 (optional effects with inference, generic words
with type variables monomorphised per use, `wack infer --write`) and M8
(sum types: `union` with `match`, generic structs and unions monomorphised
per instantiation, `option T` in the prelude) and M9 (collections: a
growable `vec T` and a hash map `map K V` in the prelude, on the
by-contents primitives `hash` and `eq`) are implemented: M0 to M9
implemented.

Licence: MIT.
