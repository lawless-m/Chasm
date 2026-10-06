# web

The browser REPL: static files only, no server code. It runs the same
compiler as the CLI, built to wasm from `crates/web`.

## Running it

```
RUSTUP_TOOLCHAIN=1.99.0 sh web/build.sh    # or plain `sh web/build.sh` if the default Rust is new enough
python3 web/serve.py                        # then open http://localhost:8000/ (Vivaldi or any current browser)
```

`build.sh` builds `wack-web` for `wasm32-unknown-unknown` and copies it to
`web/wack_web.wasm` (git-ignored).

The page uses `SharedArrayBuffer` and `Atomics.wait`, which browsers allow
only on a cross-origin isolated page. That needs two response headers:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

`file://` and plain `python3 -m http.server` do not send them;
`web/serve.py` does (stdlib only, port 8000, or the first argument). Any
static host works if it sends both headers.

On a host that cannot send them, `coi.js` registers the service worker
`coi-sw.js`, which adds the two headers to every response, and reloads the
page once. It does nothing when the page is already cross-origin isolated.

Every step module uses WasmGC (the transformed modules keep a waiting
process's frames as GC structs), so the REPL needs a browser with WasmGC,
and the node checks need node 22 or later.

## Saved program

The page keeps the program in the browser's `localStorage` (key
`wack.program`): every chunk that defines, declares, tests, forgets or
forces something without an error, in the order typed. Lines are not kept.
On load the page replays the saved chunks into the fresh session, so a
reload or a redeploy keeps your words but starts with an empty stack. A
saved chunk that no longer checks (after a compiler change) is shown with
its error and dropped.

A link can carry an example: `#code=<base64url>` in the URL holds the
UTF-8 text of a `.wack` file. After the saved program is replayed, the
page splits that text into chunks and runs each as if typed: a chunk
begins at every line that starts, at column 0, with `:`, `export`,
`declare`, `test`, `struct` or `union`, and every other line continues the
chunk before it. Loaded chunks go into the Up/Down history but are not
saved; what you type afterwards is saved as usual.

Two page commands, handled by `main.js` rather than the compiler:

- `)program` lists the saved chunks: the history, every redefinition and
  `)forget` included.
- `)clear` forgets them and reloads the page.

`)words`, a compiler command, lists the program as it stands instead:
the latest definition of each word still defined, ready to copy into a
`.wack` file (`docs/reference.md` section 1a). It is not saved, and
neither is `)test` or `)test word`, which run the tests in force.

Up and Down in the input box step through the chunks you have entered,
as in readline: Up from the first line of the box, Down from the very end,
so inside a multi-line chunk they still move the caret. Down past the
newest entry brings back what you were typing, and edits to a recalled
chunk are kept until a chunk is run. The history (500 chunks, repeats
collapsed) is kept in `localStorage` too (key `wack.history`) and survives
`)clear`.

The ⤢ button opens the box as an editor, half the window tall: Enter adds
a newline, Ctrl+Enter runs the whole text, Up and Down only move the
caret, and Esc (or ⤡) closes it. Ctrl+Enter also runs from the one-line
box, without waiting for an open definition to close.

`test/bridge.html` keeps nothing, so driving it does not touch the saved
program.

## How it works

- **Main thread** (`main.js`, `driver.js`): owns the compiler session
  (`compiler.js` over `wack_web.wasm`), the shared `WebAssembly.Memory`
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
- **Doorbell.** For host I/O, `wack.ring_enter` stores 0 in the doorbell
  cell (`DOORBELL` in the layout), posts `ring` to the main thread and
  blocks in `Atomics.wait`; the main thread services the ring, stores 1 and
  calls `Atomics.notify`. So I/O completes inline: no other process runs
  during the wait.
- **Processes** (`docs/reference.md` section 14a). The session is compiled
  with the same unwind/rewind transform as the native REPL
  (`crates/core/src/repl.rs`, `Session::new(.., unwind: true)`), and
  `drive.js` is the JavaScript twin of the native driver
  (`crates/runtime/src/native.rs`), over the scheduler `proc.js` (a state
  machine with no wasm calls; its native twin is
  `crates/runtime/src/proc.rs`). A process that must wait gets no
  completion: `ring_enter` sets the mode cell to unwinding, the transformed
  functions save their frames into `wack.frames` and return, and the driver
  keeps the chain by pid. To resume, it puts the chain back, writes the
  completion, sets rewinding and calls the same entry again. A step runs
  each test and the line as process 0, then every ready process, and ends
  when process 0 has returned and nothing is ready; processes still waiting
  stay for later steps. When nothing can run and process 0 waits, the entry
  traps `all processes blocked: ...`, naming `[line]` or the test's word. A
  trap in a spawned process goes to `processTraps` and ends only that
  process; the step does not fail. `/prog` is answered in the worker, from
  the scheduler, and never reaches the main thread; a process that kills
  itself ends with a thrown `killed` error. The worker creates the
  `anyref` globals `wack.frames` and `wack.spawn` at `init`, and every step
  module imports them.
- **Messages.** Main to worker: `init` (memory, layout), `run` (module,
  installs, table size, `refsSize`, line slot, test slots and words) and
  `render` (struct slots and layouts). Worker to main: `ring`, `done` (trap,
  test results, `processTraps`: traps in spawned processes) and `rendered`
  (struct values).
- **Structs.** References cannot live in shared memory, so a struct on the
  stack is a slot holding its index into `wack.refs`, an `anyref` table the
  worker owns. The worker creates it the first time a step module imports it
  or a step's `refs_size` is above 0. JavaScript cannot read WasmGC struct fields, so
  for the stack echo the worker calls the generated accessor words
  (`point.x`, ...) through the table; the step JSON gives each struct's fields
  and their accessor slots. A union value is read through its `tag` word,
  then the readers of that variant (`shape.circle.r`, or the instance at the
  type's arguments for `option i32`). A struct array's length is its `len`
  slot in memory.
- **Layout.** The JavaScript never hard-codes an address: `compiler.js`
  reads the layout from the compiler (`wack_core::layout::constants`).

`compiler.js`, `ring.js`, `worker-core.js`, `drive.js`, `proc.js` and `driver.js` use no DOM or
Node API, so the node checks below exercise the same code the page runs.

## The browser namespace

`/dev/cons` output goes to the page; console reads return end of input, so
`read-line` reports no more lines. `/dev/time` works. `/net/http` and
`/net/https` go through `fetch`, so a server on another origin must allow
CORS. There is no `/file` and there are no mounts: those paths return not
found.

`/prog` lists the live processes and `/prog/<pid>/ctl` takes `kill`; the
worker answers them from the scheduler.

`/local/<name>` is a flat directory of files in the page's `localStorage`,
item `wack/local/<name>`, the bytes kept as a string of char codes 0 to 255.
They outlast the tab. Every mode works; each write is saved at once, and a
write past the storage quota is -4. Reading `/local` gives directory
records. `"/net/https/host/path" "/local/name" copy` fetches a file once and
keeps it.

## Checks

Run from the repository root after `build.sh`:

```
node web/test/compiler.mjs    # the compiler wrapper
node web/test/ring.mjs        # ring servicing and the namespace
node web/test/node-repl.mjs   # end to end: driver, worker thread, shared memory, doorbell
node web/test/net.mjs         # /net/http through fetch, against a local server
node web/test/proc.mjs        # the process scheduler, without wasm
node web/test/node-structs.mjs    # structs end to end
node web/test/node-procs.mjs      # processes and channels through the REPL
node web/test/node-examples.mjs   # examples/pipeline.wack, examples/alt.wack: tests and main
```

The last three need node 22 or later (WasmGC). The same scenarios run in a
real browser:

```
sh web/test/headless.sh test/structs.html STRUCTS     # prints STRUCTS ok
sh web/test/headless.sh test/procs.html PROCS
sh web/test/headless.sh test/examples.html EXAMPLES
```

`serve.py` also serves the repository's `examples/` at `/examples/`, for
the examples page.

It serves `web/` with `serve.py` on port 8765, starts headless Vivaldi (or
the browser given as a third argument) with a throwaway profile under
`tmp/headless/`, opens the page through the debugging port (a fresh profile
opens a welcome page in place of a command-line URL) and waits for the
page's console to report.

## Driving a browser REPL through BRIDGE

`test/bridge.html` is a REPL page with no input box, for a BRIDGE broker
(remote JavaScript execution in connected browsers). It registers two
actions: `step` (one REPL step; returns the step result as the driver
reports it) and `output` (the program's console output since the last
call). A server that injects the BRIDGE client into its pages needs nothing
more; elsewhere, name the client script in the query string:
`http://localhost:8000/test/bridge.html?client=<client script URL>`. Find the
page's `connectionId` in the broker's `GET /workers` by its `path`, then:

```
curl -s -X POST -H "Authorization: Bearer $BRIDGE_TOKEN" -H "Content-Type: application/json" \
  -d '{"target": "<connectionId>", "script": "return await bridge.action(\"step\", \"3 2.5 point.new\")"}' \
  "$BRIDGE_URL/jobs/sync"
```

A client script from another origin that sends no
`Cross-Origin-Resource-Policy` header is blocked under `require-corp`, so
`serve.py` sends `Cross-Origin-Embedder-Policy: credentialless` for this
page. That keeps it cross-origin isolated in Chromium-based browsers.
