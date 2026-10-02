# Chasm

Chasm (Chuck-Wasm, after Chuck Moore) is a typed, concatenative language in
the Forth and Factor family that compiles to WebAssembly. Every word declares
its stack effect, and the checker verifies each body against it.

```
: square ( i32 -- i32 )  dup i32.mul ;
test square : 3 square -> 9

: main ( -- )  "7 squared is " print  7 square i32.to-str println ;
```

```
$ cargo run -p chasm-cli -- run examples/basics.chasm
$ cargo run -p chasm-cli -- test examples/arrays.chasm
$ cargo run -p chasm-cli -- check --json examples/strings.chasm
$ cargo run -p chasm-cli -- repl
```

`chasm repl` is interactive: type definitions and lines, see the stack after
each, redefine words live.

- `docs/reference.md`: how to write Chasm (start here)
- `ARCHITECTURE.md`: goals, runtime, milestones, decisions
- `LANGUAGE.md`: the v1 language specification
- `FUTURE.md`: what is deliberately not v1
- `examples/`: worked examples, each with tests (`docs/examples.md`: the corpus and benchmark plan)
- `web/README.md`: the REPL in the browser
- `docs/performance.md`: run speed against Rust and JavaScript (`python3 bench/run.py`)
- `docs/editors.md`: `chasm lsp` in Neovim, Helix and VS Code

Status: milestones M0 (skeleton), M1 (declared effects to wasm, CLI, native
host), M2 (interactive REPL, native and in the browser), M3 (dependency-graph
tooling: `deps`, `used-by`, `dead`, `)forget`), M4 (structs as WasmGC
structs) and M5 (whole-program export: dead words left out, unresolved words
refused, Binaryen) are implemented. M6 (polish and tooling) is in part: JSON
output everywhere, the `chasm lsp` language server, and `)force` in the REPL.

Licence: MIT.
