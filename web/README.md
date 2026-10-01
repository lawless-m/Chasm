# web

The browser REPL: static files only, no server code. It runs the same
compiler as the CLI, built to wasm from `crates/web`.

## Running it

```
RUSTUP_TOOLCHAIN=1.99.0 sh web/build.sh    # or plain `sh web/build.sh` if the default Rust is new enough
python3 web/serve.py                        # then open http://localhost:8000/ (Vivaldi or any current browser)
```

`build.sh` builds `chasm-web` for `wasm32-unknown-unknown` and copies it to
`web/chasm_web.wasm` (git-ignored).

The page uses `SharedArrayBuffer` and `Atomics.wait`, which browsers allow
only on a cross-origin isolated page. That needs two response headers:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

`file://` and plain `python3 -m http.server` do not send them;
`web/serve.py` does (stdlib only, port 8000, or the first argument). Any
static host works if it sends both headers.

## How it works

- **Main thread** (`main.js`, `driver.js`): owns the compiler session
  (`compiler.js` over `chasm_web.wasm`), the shared `WebAssembly.Memory`
  (4 MiB initial, 64 MiB maximum) and the I/O ring (`ring.js`). Each step
  follows the session's host contract (`crates/core/src/repl.rs`): it writes
  the step's literals at the heap pointer, sends the step module to the
  worker, then commits the stack, or restores the memory data stack if the
  line trapped.
- **Worker** (`worker.js`, `worker-core.js`): owns the funcref table and
  every step's instance, since tables cannot be shared between threads. It
  instantiates each step module against the shared memory and its table,
  installs the functions at their slots, runs the step's tests and line, and
  reports traps from the trap cells.
- **Doorbell.** The compiled code's one import, `chasm.ring_enter`, stores 0
  in the doorbell cell (`DOORBELL` in the layout), posts `ring` to the main
  thread and blocks in `Atomics.wait`. The main thread services the ring,
  stores 1 and calls `Atomics.notify`.
- **Messages.** Main to worker: `init` (memory, layout) and `run` (module,
  installs, table size, line slot, test slots). Worker to main: `ring` and
  `done` (trap, test results).
- **Layout.** The JavaScript never hard-codes an address: `compiler.js`
  reads the layout from the compiler (`chasm_core::layout::constants`).

`compiler.js`, `ring.js`, `worker-core.js` and `driver.js` use no DOM or
Node API, so the node checks below exercise the same code the page runs.

## The browser namespace

`/dev/cons` output goes to the page; console reads return end of input, so
`read-line` reports no more lines. `/dev/time` works. There is no `/file`,
no mounts and no `/net`: those paths return not found.

## Checks

Run from the repository root after `build.sh`:

```
node web/test/compiler.mjs    # the compiler wrapper
node web/test/ring.mjs        # ring servicing and the namespace
node web/test/node-repl.mjs   # end to end: driver, worker thread, shared memory, doorbell
```
