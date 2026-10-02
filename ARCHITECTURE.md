# Chasm: Architecture and Milestones

Status: draft v0.16 (M0 to M6 implemented; decisions in sections 13 to 18). **Chasm** (Chuck-Wasm, after Chuck Moore) is a typed, concatenative language that compiles to WebAssembly, with an interactive REPL, written in Rust. Source files use the `.chasm` extension; the CLI binary is `chasm`.

## 1. Goals

- A small Forth/Factor-like concatenative language whose words map almost one-to-one onto wasm.
- Every word carries a **declared stack effect**. The checker is the single source of truth.
- An **interactive workflow**: type words, see them run, redefine them live.
- **Contract first**: declare a word's effect (and optional tests) before writing its body.
- The compiler core runs **both natively and as wasm in the browser** from the same source.
- Designed so that **Claude Code can write and fix programs** from compiler feedback alone.

Non-goals for v1: inference, closures, the Component Model, multi-threading. See `FUTURE.md` for how these would fit. Arrays and functions as values (without capture) **are** v1, and structs are milestone M4, as WasmGC structs (section 15).

Licence: MIT.

## 2. Language target

- Primary implementation language: **Rust**. Wasm tooling (`wasm-encoder`, `wasmparser`) is strongest there.
- Output: standard wasm modules, optionally optimised with Binaryen at export time.
- Assumed wasm features: multi-value, reference types (`table.set`), bulk memory. These fix the minimum engine version; all current browsers and runtimes qualify.
- Stack effects look like `( i32 i32 -- i32 )` and map directly to wasm function types. Multi-value returns are used for multiple outputs.

## 3. Crate layout

| Crate | Role | Notes |
|---|---|---|
| `core` | Lexer, AST, effect checker, dependency graph, wasm emitter, word database | **No I/O.** Builds to native and wasm32 unchanged. |
| `runtime` | Host-side support: data stack, function table, imports (print, memory, time) | Thin; separate native and browser implementations behind one trait. |
| `cli` | `check`, `build`, `run`, `deps`, `used-by`, `unresolved`, `dead`, `words`, `test`, `repl`, `lsp` | Text and JSON output on every command; `lsp` speaks JSON-RPC. |
| `web` | `web/`: the browser REPL, over `crates/web` (`chasm-web`: the core's REPL session behind a C ABI, compiled to wasm32) | Static files only, no server. |
| `docs` | Language reference, worked examples | Treated as part of the product (see section 9). |

Rule: anything the CLI, REPL and exporter all need lives in `core`.

## 4. Core pipeline

1. **Lex**: whitespace-separated tokens, with comments and stack-trace annotations kept.
2. **Parse**: word definitions, declarations, tests. Effect is an **optional field** in the AST from day one (v1 rejects its absence).
3. **Resolve**: names to word ids; edges recorded in the dependency graph here (not at parse time).
4. **Check**: verify each body against its declared effect. Report expected versus actual stack at the failing token.
5. **Emit**: one wasm function per word, typed from its effect.
6. **Link/instantiate**: via the shared table and memory (REPL) or direct calls (export).

Type representation: an effect is a list of input types and a list of output types, with **space reserved for a "rest of stack" row variable**. Not implemented in v1, but costly to retrofit, so the data structure allows it.

## 5. Runtime layer

- **Two stacks, with clear roles.** Words pass values on the native wasm operand stack: a word's effect is its wasm function type, and words call each other natively. The **data stack in linear memory** holds only the REPL's state between lines. A REPL line compiles to an anonymous word that loads its inputs from the memory stack, runs, and stores its outputs back. Exported programs never touch the memory stack unless they use it explicitly.
- **Shared function table** (`funcref`). Each word owns a slot. Callers use `call_indirect`.
- **Shared linear memory**, imported by every module.
- **Host imports**: four I/O words over a namespace. See section 5d.
- **Per-step modules in the REPL.** Each REPL step compiles the words it defines (named words, quotation values, test thunks, and the step's anonymous line word) into one small module that imports the shared memory and table. Shape and measured cost in section 14.
- **Memory layout.** One linear memory, fixed regions at fixed offsets, agreed by the REPL, host and exporter:

  | Offset | Region | Notes |
  |---|---|---|
  | 0 to 64 KiB | Reserved | Address 0 is never valid; loads and stores here are a bug. |
  | 64 KiB to 128 KiB | I/O ring | Submission and completion queues (section 5d). |
  | 128 KiB to 1 MiB | Data stack | Fixed size. Overflow traps with a message. |
  | 1 MiB upward | Literals, then heap | Read-only literals first, bump heap after, growing by `memory.grow`. |

  Initial memory is 4 MiB (64 pages). These numbers are v1 defaults, not promises; they live in one place in `core`.
- **Memory ownership.** v1 uses a bump allocator with no free. Strings and other non-numeric data (`LANGUAGE.md` sections 4 and 4a) build on it. A real allocator is a later concern and must not leak into the language design.

## 5a. Locals

Wasm provides locals for free, and the language uses them at two levels:

- **Named locals as sugar.** A word may bind values off the stack into word-scoped names with Factor's `:> name` (`:> name!` for mutable). Each local is typed by whatever was on the stack when it was bound, so the checker needs no new theory, and the emitter maps them directly to `local.get`/`local.set`. The word's effect is unchanged, so the interface story is untouched. This is the single biggest aid to writing correct code, human or machine: bind the inputs, and the stack only matters at the edges.
- **Compiler-only locals.** The emitter uses wasm locals internally to implement shuffles such as `rot` and `pick` that have no cheap operand-stack encoding. Users never see these.

v1 limits: locals are word-scoped and not captured by quotations (that would mean closures). Mutation is explicit via the `!` suffix. Shuffle words (`dup`, `swap`, `over`, `drop`) remain idiomatic for short words; locals are the expected choice beyond three or so values.

## 5b. Polymorphism

Wasm has no polymorphic instructions, and a word's effect is its wasm function type, so **user words are monomorphic**. The only polymorphism in the language is built into the shuffle primitives.

- **Shuffle primitives** (`dup`, `swap`, `drop`, `over`, `rot`, `nip`, `tuck`, `2dup`) and **collection combinators** (`each`, `map`, `filter`, `fold`) have effects written with type variables, for example `( a b -- b a )`. Checking is forward and every user word is concretely typed, so the stack is always concrete when the checker meets one of these. It instantiates the primitive at the types present and the emitter produces the matching code: `dup` on `i32` is `local.tee` into a compiler-only `i32` local, `swap` is two locals, and so on.
- **No type variables in user effects.** A word that would need `( a b -- a b a b )` must instead be written once per type (`2dup-i32`, `2dup-f64`). Tedium is chosen over magic: every user word has a real wasm function type, a real table slot, and a single node in the dependency graph.
- **Later, if wanted:** monomorphisation, where user effects may carry type variables and each call site gets its own specialised instance. That needs per-instance table slots and graph nodes, and shares machinery with M7 inference, so it is deferred to that point and may never be needed.

## 5c. Control flow

Wasm has only structured control flow (`block`, `loop`, `if`, branches to enclosing labels). The language uses **Factor-style quotations and combinators**, which map onto that one-to-one.

**A quotation under a combinator is syntax.** A `[ ... ]` that is the argument of a built-in combinator is inlined as a wasm block in place: no table slot, no indirect call, and locals of the enclosing word are visible inside it because it is part of that word's body. This covers control flow and the collection combinators (`each`, `map`, `filter`, `fold`, in `LANGUAGE.md` 4a).

**A quotation anywhere else is a value** (`LANGUAGE.md` 7a): a table index with a quotation type, called via `call`. Values do not capture locals.

Combinators in v1:

| Word | Shape | Wasm | Checking rule |
|---|---|---|---|
| `if` | `cond [ then ] [ else ] if` | `if`/`else` | Both branches checked from the same concrete stack and must leave the same stack; that shape is the block type. |
| `when` / `unless` | `cond [ body ] when` | `if` | Body's net effect must be the identity. |
| `while` | `[ cond ] [ body ] while` | `block`/`loop` | Body's net effect is the identity (the loop invariant). Condition leaves one extra `i32` on top. |
| `until` | `[ body ] [ cond ] until` | `block`/`loop` | As `while`, condition tested after the body. |
| `times` | `n [ body ] times` | `block`/`loop` | Counter held in a compiler-only local; the index is pushed each iteration. Body's effect is `( i32 -- )`. |
| `leave` | inside a loop body | `br` to the loop's outer block | Stack at `leave` must match the loop's exit shape. |

Conditions are `i32`, zero false, as in wasm. Errors name both branches: "branches of `if` disagree: then-branch leaves `( i32 )`, else-branch leaves `( i32 i32 )`".

Later, each a separate decision: `case` over an `i32` via `br_table`; closures.

## 5d. I/O

One interface for everything, in the Plan 9 spirit: **four words over a namespace**.

| Word | Effect | Notes |
|---|---|---|
| `host.open` | `( str i32 -- i32 )` | Path and mode; returns a handle, or a negative error code. |
| `host.read` | `( i32 i32 i32 -- i32 )` | Handle, buffer address, length; returns bytes read, 0 at end, negative on error. |
| `host.write` | `( i32 i32 i32 -- i32 )` | Same shape. |
| `host.close` | `( i32 -- i32 )` | |

These are the only host imports. **Everything else is a path.** Adding a capability means adding a path, not a word.

| Path | Meaning |
|---|---|
| `/dev/cons` | Console. `print` is library code that writes here. |
| `/dev/time` | Read returns a timestamp. |
| `/net/http/<host>/<path>` | Opens an HTTP request. A write sends header lines, an empty line and a body; a read receives the response body (`LANGUAGE.md` 17). |
| `/file/...` | Host filesystem, where one exists. |
| `/mnt/<name>/...` | Mounted trees, including 9p servers. |

**Directories.** Reading a directory handle returns entries in one fixed encoding: a length-prefixed record per entry (name, size, is-dir flag). It is the same everywhere, so a directory-walking word works on local files, the browser's in-memory tree, or a remote 9p mount alike.

**Mounts.** Host configuration: the CLI takes `--mount name=DIR` or `--mount name=9p://host:port` (also written `/mnt/name=...`), natively. 9p from the browser, over WebSocket, is deferred. A `host.mount` word may come later.

**Transport: a submission/completion ring from day one.** Each `host.*` word writes a request into a ring in linear memory, waits for the completion, and returns the result. The words look synchronous to the language; asynchrony lives entirely in the host.

- **Native host**: services the ring with files, an HTTP client, a 9p client, and optionally real io_uring as the backend.
- **Browser host**: the wasm instance runs in a Web Worker. The main thread services the ring with `fetch`, WebSocket (for 9p) and the DOM; the worker blocks on `Atomics.wait` on the doorbell cell until the main thread stores 1 and notifies. This is what makes blocking reads possible in the browser at all.

**What this buys:**
- `check` can list the paths a word opens, so a host lacking `/file` can say so before running.
- Tests can mount a fake namespace: `/dev/cons` captured into a buffer, `/net/http/...` answered from a fixture.
- WASI export is a mapping of four words and a path table, not an API translation.
- One I/O idiom for Claude Code to learn, and it is the Unix one.

**Cost:** the runtime layer is built properly in M1 rather than later, and the browser REPL depends on the worker (section 14). Error codes and the directory record encoding are specified in `LANGUAGE.md`.

## 5e. Entry points and exports

- `run` calls `main ( -- )`. A program without `main` can be checked and built but not run.
- A definition may carry the `export` attribute: `export : name ( effect ) ... ;`. Exported words become wasm exports in the exported module, and are the roots for reachability (dead-word trimming, the unresolved check). `main`, if present, is always a root.
- The source is self-describing; the CLI does not take an export list.

## 5f. REPL rules

- A typed line compiles to an **anonymous word**. Its effect is not declared: the checker runs forward from the current types of the memory data stack, which are always concrete, and the line is accepted if it checks. This is the only place v1 derives an effect, and it is plain forward checking, not inference.
- After each line the REPL prints the resulting stack with types.
- If a line **traps**, the data stack is restored to its state before the line, and the trap message and word are printed.
- Definitions, declarations and tests typed at the REPL behave exactly as in a file.

## 6. Declare, define, redefine

One rule governs all three: **a word's declared effect is its interface and can only change deliberately.**

- `declare name ( effect )` creates a table slot with the right wasm function type. The body is a trap that reports `unresolved word <name>`. Callers type-check, compile and run until they hit it.
- Defining a word fills the slot **only if the declared effect matches exactly**. Mismatch is an error.
- Redefining a word with the **same effect** swaps the table entry. Existing callers pick up the new body.
- Redefining with a **different effect is rejected** in v1. The error lists all dependants from the dependency graph.
- Declaring twice: identical is a no-op, conflicting is an error.
- Optional **contract tests** (`test word : body -> expected`) can sit with a declaration. They stay pending until a body exists, then become its first check.
- **Forward references require a declared stub.** No bare undeclared names in v1.
- **Export refuses unresolved words that are reachable** from an exported entry point, and lists them all. Unreachable stubs are dropped with the rest of the dead words.

The REPL command `)force` changes an effect deliberately, re-checking every dependant and refusing the whole change if any breaks (section 18).

## 7. Dependency graph

Lives in `core`. Stored as two maps, **callers-of** and **callees-of**, updated on every define, redefine and declare.

Edge kinds:
- **Call**: a direct call.
- **Address-taken**: the word's address escapes as a value (like a Forth tick). Such a word cannot be safely inlined or removed, and table indices on the stack can go stale.

Node state: **resolved** or **unresolved** (declared, no body yet).

Uses:
- Rejection errors that name affected dependants.
- Safe `forget`: refuse, or list what would be orphaned.
- Dead-word detection and trimming at export.
- Cycle detection via strongly connected components. In v1 every word is declared, so this matters only for M7 inference, where recursive and mutually recursive words must keep declared effects.
- Incremental checking after a body edit.
- `deps`, `used-by` and `unresolved` commands with JSON output for tooling and Claude Code.

## 8. Export (hybrid mode)

The REPL uses indirect calls. The export step does whole-program compilation: direct calls and dead-word removal, then Binaryen optimisation via an external `wasm-opt` binary (not a crate dependency, so the browser build stays light), which also inlines small words; Chasm has no inliner of its own (section 17). It fails if any reachable declared word is unresolved. Same source, two back ends; the checker is shared.

## 9. Designing for Claude Code

- **Machine-readable errors**: expected effect, actual effect, exact token, in text and JSON.
- **Checked stack assertions** inside bodies, written in types (for example `( i32 i32 )`), which the checker verifies. Syntax in `LANGUAGE.md` section 9.
- **Fast `check` command** that validates without running, so the fix loop is nearly free.
- **Small, regular core**: consistent names (`i32.add`), few primitives, no clever abbreviations.
- **Tests in the language**, written beside each word, so behaviour is verified as well as types.
- **Language reference and worked examples** kept in the repo and kept current. A new language is not in any model's training data, so these carry most of the weight. Stay close to Forth and Factor conventions where possible.
- **`unresolved` as a to-do list**: Claude Code works through declared stubs one at a time against their contracts.

## 9a. Machine-readable output

Principles; exact fields are settled in M1 and generated from the Rust types (`schemars`) if a formal schema is ever wanted.

- **One top-level shape for every command**: `{ "schema": 1, "ok": bool, "command": "check", "diagnostics": [...], "results": {...} }`.
- **Every diagnostic** has a stable `code` (for example `E_EFFECT_MISMATCH`, `E_UNRESOLVED`, `E_REDEFINE_EFFECT`), a `severity`, a `message`, and a `location` (file, line, column, token). Codes are the contract; message wording may change.
- **Fix-enabling fields** where they apply: `expected` and `actual` as arrays of type names for stack errors; `dependants` for rejected redefinitions; `declared_effect` for unresolved words.
- **Test results**: `{ "test", "word", "status": "pass" | "fail" | "pending", "expected", "actual" }`.
- **Text output is rendered from the JSON**, never written separately, so the two cannot drift.

## 10. Milestones

**M0: Skeleton.** Workspace, CI, `core` builds native and wasm32. Language reference stub.

**M1: Declared effects to wasm.** Lexer, parser, effect checker, emitter, `check`/`build`/`run` in the CLI. Primitive set from the language spec, including arrays and the inlined collection combinators. Named locals. `declare` stubs with trap messages and the `unresolved` command (the dependency graph lives in `core` from the start). Tests in the language. Structured errors. Native host with the ring transport, `/dev/cons`, `/dev/time`, `/file`.

**M2: Interactive REPL.** Data stack in linear memory, shared table, per-word modules, redefinition with matching effects. Native REPL first, then browser with the Web Worker host. **Measure the compile-edit cycle** and record it.

**M3: Dependency graph tooling and functions as values.** Contract tests, `deps`, `used-by`, rejection errors that name dependants, safe `forget`, dead-word detection. `'word`, quotation values, quotation types in effects, `call`, address-taken edges.

**M4: Structs.** `struct` declarations lowered to WasmGC struct types, with generated words and engine garbage collection (section 15). Struct values on the REPL stack through a reference table, in the native and browser REPLs. Arrays of structs. The browser checks move to node 22.

**M5: Hybrid export.** Whole-program compilation, direct calls, dead-word removal, Binaryen. Refuse unresolved words. Inlining is left to Binaryen (section 17).

**M6: Polish and tooling.** JSON output everywhere, editor integration (`chasm lsp`), expanded examples, force-redefine with cascade (`)force`), `/net/http` with request headers natively and in the browser, 9p mounts, WASI export mapping (`build --wasi`). An io_uring native backend was measured and not built (section 18).

**M7: Inference (optional).** Elaboration pass in front of the checker producing annotations, with row variables. Soundness unaffected because the checker re-verifies. Start with straight-line words made only of primitives, then add unification.

M1 to M3 can overlap; the graph and stub data structures are part of M1 so that M3 is tooling only.

## 11. Open questions

Settled questions live in `LANGUAGE.md` (primitive set, numeric types, strings, effect syntax, tests; M1 decisions in its section 12); the policy for dependants broken by a forced effect change and the `/net/http` semantics are settled in section 18 and `LANGUAGE.md` section 17. Still open:

1. Load/store alignment and offset immediates (`LANGUAGE.md` open item 1): v1 is natural alignment, offset 0.
2. Module or namespace structure for libraries (`LANGUAGE.md` open item 2); a flat dictionary until it hurts.
3. Integer overflow. The primitives are wasm's (`LANGUAGE.md` section 2), so integer arithmetic wraps silently: `21 fact` is a well-typed wrong answer (`examples/factorial.chasm`). Chasm's safety is memory and type safety, not arithmetic safety, and the effect checker tracks types, not ranges. If checked arithmetic is wanted, the candidate is library words in the prelude (e.g. a trapping `i64.mul?`) beside the unchanged primitives, keeping one primitive to one instruction. Open: whether to add them, their names, and whether examples should prefer them.

Row variables in effects are an M7 matter, not a v1 question; the type representation leaves room for them (section 4).

## 12. Hardware notes

Heavy native batch work (large test corpora, benchmarks, Binaryen runs over big programs) can use the work machine (3090, dual Xeon, 64 GB). Day-to-day development runs comfortably on the home machine (4070, 16 GB, Debian). Nothing in the design needs a GPU.

## 13. Decisions taken in M0 and M1

1. **Crates.** `crates/core` (package `chasm-core`), `crates/runtime` (`chasm-runtime`, wasmtime behind the default `native` feature), `crates/cli` (`chasm-cli`, binary `chasm`). `crates/web` (`chasm-web`) is the browser's compiler and `web/` the browser REPL (section 14). `core` and `chasm-web` build for `wasm32-unknown-unknown`, and CI checks that.
2. **One doorbell import.** The compiled module imports exactly one host function, `chasm.ring_enter ( -- )`. The four `host.*` words are compiled code: each writes a submission entry into the ring, calls the doorbell, and takes one completion. The native host services the ring synchronously inside the doorbell; the browser host posts to the main thread and `Atomics.wait`s on the doorbell cell. Ring layout (in `core::layout`): heads and tails at 64 KiB; 256 submission entries of 32 bytes (`op, user, a0, a1, a2`); 256 completion entries of 16 bytes (`user, result`). Opcodes: 1 open, 2 read, 3 write, 4 close.
3. **Runtime cells** in the reserved region: trap message address and length at `0x100`/`0x104`, trapping word at `0x108`/`0x10C`, heap pointer at `0x110`, data stack pointer at `0x114`, browser doorbell at `0x118`. Address 0 stays invalid.
4. **Trap messages.** `trap`, bounds checks, unresolved stubs and out-of-memory call a runtime helper that writes the message and word into those cells, then executes `unreachable`. The host reads them back. Plain wasm traps (division by zero) are named from the module's name section via the backtrace.
5. **M1 module shape.** One module per program: runtime helpers (`rt.alloc`, `rt.trap`, `rt.ring`), then one function per word in definition order. Calls are direct. Every word also owns a slot in a funcref table (slot = word index) for `'word` and `call` (`call_indirect`). The module defines and exports its memory; the REPL's step modules import it instead (section 14). `check`, `build`, `run` and `test` use this whole-program path; the REPL path sits beside it. Literals start at 1 MiB, and the heap follows them.
6. **Functions as values** (`'word`, quotation values, `call`, address-taken edges) were cheap on top of the table, so they landed in M1 rather than M3.
7. **Two-pass checking.** Each body is walked twice by the same checker: a checking pass that settles type variables, then an emitting pass with the final substitution. Blocks take the whole checker stack as parameters (multi-value), so a quotation under a combinator can reach any value below it.
8. **Text from JSON.** Every CLI command builds the JSON report; the text output is rendered from that JSON value.
9. **Host namespace.** `/file/<path>` is the host path `/<path>` (off with `--no-file`). `--mount name=DIR` mounts a local directory at `/mnt/name`, `--mount name=9p://host:port` a 9p server, and `..` is refused under mounts. `--no-net` hides `/net`, which then returns "not supported", as other `/net/...` services do.

## 14. Decisions taken in M2

1. **Compile-edit cycle (measured).** Native REPL, release build, wasmtime 49 with Cranelift, on the home machine (AMD Ryzen 5 5500, Debian), 2026-10-01. Input: `examples/basics.chasm` followed by 20 alternating redefinitions of `square`, 20 lines `3 square drop` and 10 lines `"abc" "def" str.concat drop`, fed as `RUSTUP_TOOLCHAIN=1.99.0 cargo run -q --release -p chasm-cli -- repl --json < tmp/measure.chasm > tmp/measure.jsonl` (67 chunks; timings from `results.timing`). Medians (maximum in brackets):

   | Step | Compile (checker + step module) | Instantiate (place literals, Cranelift, instantiate, table installs) | Run |
   |---|---|---|---|
   | Definition (27 chunks) | 55 µs (77 µs) | 3.0 ms (4.1 ms) | 3 µs |
   | Redefinition of `square` (21) | 53 µs (74 µs) | 3.0 ms (3.3 ms) | 3 µs |
   | Bare line (30) | 56 µs (168 µs) | 3.2 ms (3.9 ms) | 2 µs |

   Start-up, including engine creation and installing the prelude as one step module: 11 ms wall time for `chasm repl < /dev/null`. A step costs about 3 ms end to end, almost all of it in wasmtime compiling and instantiating the step module; the checker and assembler take about 50 µs. That meets the "small enough for instant feedback" expectation of section 5: a step is well under the roughly 100 ms at which a delay becomes noticeable.
2. **Step modules.** `module::assemble_step` builds one module per REPL step. It imports `chasm.ring_enter`, `chasm.memory` and `chasm.table` (a funcref table), carries its own copy of the runtime helpers `rt.alloc`, `rt.trap` and `rt.ring` at function indices 1 to 3, and exports each of its words as `w<id>`. The host sets table slot `id` to that export, so the slot of a word is its id. Calls between words are `call_indirect` through the table (`Ctx::indirect_calls`), which is how a redefinition reaches existing callers. A step module of a session with structs also imports `chasm.refs`, an `anyref` table (section 16). A step module has no memory, table, element or data section. The browser variant declares the memory import shared, with 4 MiB initial and 64 MiB maximum (`SHARED_MAX_PAGES`).
3. **Session and host contract.** `core::repl::Session` holds the word database and the types on the memory data stack. `step(text, heap_ptr)` returns a `Step`: diagnostics, module bytes, installs, table size, literal bytes and address, defined words, the line, and the tests to run. The host writes the literal bytes at the heap pointer and moves the heap pointer past them (8-aligned), grows the table, instantiates the module, sets the slots, runs the tests, then runs the line. On success it commits the line's output types as the session's stack. A step with errors still places its literals and installs the words it added, so a word declared or failing to check is installed as its unresolved stub, as in a file; it runs no line and no tests.
4. **Memory data stack.** One 8-byte slot per wasm value from `DATA_STACK_BASE` (`STACK_SLOT`): `i32` and `f32` in the low 4 bytes, `i64` and `f64` the whole slot; `str` and `array T` take two slots, address then length. A struct takes one slot holding an index into `chasm.refs`; an `array <struct>` takes three (that index for its GC array, then start and length). `DATA_STACK_PTR` holds the next free slot. A line compiles to a function of wasm type `( ) -> ( )` named `[line N]` (`Mode::Line`, `WordKind::Line`): its prologue pops its inputs off the memory stack, its epilogue pushes its outputs and traps with "data stack overflow" past `DATA_STACK_END`. Every line takes a table slot that is never freed.
5. **Literals at the REPL.** `Ctx::begin_literals(heap_ptr)` starts each step's literal window at the heap pointer; literals are deduplicated within a step only.
6. **Traps.** Before a line runs, the host saves the bytes of the memory stack and `DATA_STACK_PTR`; after a trap it restores both and the session keeps its old stack. It then clears `TRAP_MSG_LEN` so the next trap is reported cleanly.
7. **Tests at the REPL.** A test runs as soon as its word has a body, by calling its thunk through the table in the shared instance. A test of a declared word waits and runs when the word gets a body, and a word's tests run again whenever it is redefined. The console output of tests is not captured.
8. **`chasm repl`.** Reads stdin line by line, continuing a chunk while a definition or quotation is open (`repl::needs_more`), and builds one report per chunk; `--json` prints each as one line, with the program's console captured into `results.output`. In text mode the program's console is the terminal and shares stdin with the REPL.
9. **Browser.** `crates/web` exposes the session through a hand-written C ABI (`chasm_new`, `chasm_step`, `chasm_line_done`, `chasm_needs_more`, result buffers), not wasm-bindgen: the API is small, and the installed wasm-bindgen CLI did not match the crate version. The main thread owns the compiler session, the shared memory and the ring (`web/ring.js`, the same semantics as `service_ring`). The worker owns the funcref table and every step's instance, because tables cannot be shared between threads. `ring_enter` stores 0 in the doorbell, posts `ring` and waits; the main thread services the ring, stores 1 and notifies. JavaScript reads the layout from the compiler (`layout::constants`). The browser namespace has `/dev/cons` output and `/dev/time` only; console reads return end of input. The page needs cross-origin isolation (`Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Embedder-Policy: require-corp`), which `web/serve.py` provides.

## 15. Decisions taken in M3

1. **Structs are WasmGC structs** (milestone M4). The by-reference linear-memory design in `FUTURE.md` needed an allocator with `free`, and a manual `free` allows use-after-free, which breaks type safety; WasmGC structs are collected by the engine and cannot dangle or be misread. A spike (wasmtime 49 and Vivaldi, October 2026) confirmed what the REPL needs:
   - Each step module declares the struct types it uses; equivalent declarations are the same type across modules, so a struct made in one step is read in a later one through the shared table.
   - A struct on the REPL's memory data stack is one slot holding an index into a shared `anyref` table (`chasm.refs`); a line casts it back with `ref.cast`. The table is a GC root, so the value survives collection. The host reads fields for the stack echo.
   - A cast to a different struct type traps (`cast failure`), even with the same field types.
   - Wasmtime runs with `Collector::Copying`: 200 million short-lived structs took 0.26 s with flat memory, against 22 s with the deferred reference-counting collector. Vivaldi took 54 ms for 50 million.
   - Node 20's V8 predates the final WasmGC encoding, so the node checks need node 22 or later.

   The source form and generated words are as first sketched: one declaration, every access a dictionary word with an effect, no hand-written offsets.

   ```
   struct point  x: i32  y: f64
   ```

   | Generated word | Effect | Lowering |
   |---|---|---|
   | `point.new` | `( i32 f64 -- point )` | `struct.new` |
   | `point.x` | `( point -- i32 )` | `struct.get` |
   | `point.x!` | `( point i32 -- )` | `struct.set` |

   `!` means write, as for locals. Generated words are ordinary words: `words` lists them, they appear in the dependency graph. Strings and arrays stay in linear memory; a `str` or `array T` field is two `i32` fields. Exported modules (M5) require an engine with GC.

2. **`)forget` is a REPL command, not language.** A line starting with `)` is handled by `Session` before parsing, so a file can never forget anything, including a file loaded into a session. It removes the word's name and its tests (the session records their indices so they never run again) and renames the word `[forgotten name]` with no edges. Its table slot is never reused, so a function value of it on the stack keeps calling the old code with the old type. It is refused with `E_FORGET` while a named word, a quotation inside one, or another word's test uses the word; REPL lines do not count. Primitives, prelude words and struct-generated words are refused.
3. **Dead words** are computed over the whole program from its roots, `main` and the `export` words, along call and address-taken edges (`Compilation::dead`, `chasm dead`). Tests are not roots. A program with no root reports none rather than everything, so a library file stays quiet. Prelude and struct-generated words are never reported; `WordInfo.generated` marks the latter. Trimming dead words at export is M5.

## 16. Decisions taken in M4

1. **Type table.** `Ctx.types` holds function, struct and struct-array definitions (`TypeDef`). Each struct is emitted as one rec group of two: the struct, then `array (mut (ref null struct))`, the storage of `array <struct>`. The table is append-only and every step module emits all of it, so a type keeps its index across REPL steps, and equivalent rec groups in separately compiled modules are the same type. A struct may name only earlier structs or itself, so referenced types always come first.
2. **Arrays of structs are views** `( ref start len )` over a WasmGC array, three wasm values, so they slice and share storage like linear arrays. `array.new` makes the GC array (`array.new_default`, null elements); `array.at`, `array.at!` and `array.slice` check bounds with the same trap messages as linear arrays; `map` and `filter` build a fresh GC array viewed from 0.
3. **The REPL stack and `chasm.refs`.** Step modules of a session with structs import `chasm.refs`, table 1, an `anyref` table. A line's epilogue stores each reference in `chasm.refs` at its own slot index (counted from `DATA_STACK_BASE`) and writes that index into the slot; the prologue reads it back with `table.get` and `ref.cast`. The table is a GC root, so values on the stack survive collection, and a trap needs no extra restore. `Step.refs_size` tells the host how far to grow the table. Modules of sessions without structs are byte-identical to M2's, and programs without structs compile byte-identically to M3's.
4. **Collector.** Wasmtime is built with the `gc` and `gc-copying` features, and `native::engine()` configures WasmGC with `Collector::Copying` for both the runner and the REPL (200 million short-lived structs: 0.26 s, against 22 s with deferred reference counting).
5. **Stack echo.** Core renders `Value::Struct`, `Null` and `Opaque`; a struct array's length is its `len` slot. The native host reads struct fields through the wasmtime GC API inside a `RootScope`. The browser worker cannot read WasmGC fields from JavaScript, so it calls the generated accessor words through the funcref table, answering a `render` message with `rendered`; the step JSON carries each struct's fields and accessor slots. Nesting shows three levels, then `name{...}`.
6. **Browser.** The worker creates `chasm.refs` the first time a step module imports it or a step needs it (`refsSize`), so engines without WasmGC run sessions without structs unchanged.
7. **Export.** The whole-program path has no refs table. A module that uses structs needs a GC-capable engine (M5).
8. **Checks.** CI runs the web checks on node 22 and adds `node web/test/node-structs.mjs`; node 20 cannot run WasmGC. Locally, `sh web/test/headless.sh test/structs.html STRUCTS` runs the same scenario in headless Vivaldi. It opens the page through the debugging port, because a fresh browser profile opens a welcome page at start-up in place of the command-line URL.

## 17. Decisions taken in M5

1. **Export mode.** `Options.export` (set by `build` and `run`) computes the words `main` and the `export` words reach along call and address-taken edges (`live_words`). Every reachable word declared without a body is refused with `E_UNRESOLVED`, listing its callers; unreachable stubs are dropped silently. A program with no root keeps every word and refuses nothing.
2. **Dead words are left out of the module** (`ModuleOptions.live`). Live words are renumbered and direct `call`s remapped, but each keeps its table slot (its id), so a `'word` value is the same number either way; a dead word's slot stays null. Dropping the prelude words a program does not use takes sieve from 3792 to 1992 bytes.
3. **Binaryen is an external `wasm-opt`**, version 121 or later (120 cannot read blocks with inputs, which the emitter uses); CI pins release 133 by checksum. `build` runs `wasm-opt -O3 -g` with exactly the features Chasm emits, validates the result, and falls back to the unoptimised module with a note if the tool is missing, fails, or produces an invalid module. With dead words already gone it saves about a quarter more (sieve: 1992 to 1517 bytes), by inlining small words and tidying locals.
4. **`run` does not use `wasm-opt` by default** (`--opt` turns it on). Measured under wasmtime (median of five, ms, sieve / mandelbrot / n-queens / quicksort): none 92.9 / 113.9 / 110.5 / 92.7; `-O3` 108.9 / 126.8 / 108.2 / 82.8; `-O1`, `-O2`, `-O4` and `-Os` are no better overall. Cranelift optimises at load time, and Binaryen's passes are tuned for V8.
5. **No inliner of our own.** Binaryen's inliner alone (`--inlining-optimizing`) changed nothing measurable under wasmtime (94.4 / 114.5 / 110.1 / 93.7), so a Chasm inliner would cost effort for no speed. Revisit if a browser measurement shows a gain.

## 18. Decisions taken in M6

1. **`)force` refuses and lists.** The chunk's definitions and every dependant of a changed word (named words, quotations in them, other words' tests; REPL lines do not count) are checked in a transaction on the session (`Session::force`). Any error restores the snapshot and reports `E_FORCE` with the broken dependants, followed by their errors. Success commits everything, installs the rebuilt words at their slots and re-runs their tests. Only the words that call a changed word directly, or through quotations, are re-checked: their own effects are unchanged, so their callers are unaffected.
2. **A forced word takes a fresh id and table slot.** The old slot keeps the old code and type, so a function value already on the memory data stack stays type-safe, as with `)forget`. Dependants keep their slots, because their effects are unchanged. The forced word's own tests are dropped; they tested the old effect.
3. **The session keeps source.** `Session` keeps each user word's and test's source item (`sources`, `test_items`, the latter keyed by test index) to re-check from, and `Ctx` and `Program` are `Clone` for the snapshot.
4. **A `)force` chunk continues until an empty line** (`needs_more`); the browser asks with the pending newline included, as the native REPL does, so one empty line ends it in both.
5. **Editor integration is `chasm lsp`**, a Language Server Protocol server over stdio in the CLI crate (`lsp-server`, `lsp-types`): diagnostics with their Chasm codes on open, change and save, hover with effects, and go-to-definition, all for the single open document with the prelude. No editor-specific extension; `docs/editors.md` configures Neovim, Helix and VS Code's generic client. The core stays free of I/O.
6. **JSON everywhere.** Every command prints the same `{schema, ok, command, diagnostics, results}` report, including clap usage errors under `--json` (`E_USAGE`), pinned by the test `json_report_shape_for_every_command`; `docs/reference.md` section 1 lists each command's `results`. `lsp` is the exception: it speaks JSON-RPC.
7. **HTTP natively** is `ureq` with `rustls` behind the namespace (`crates/runtime/src/net.rs`). The request is sent at the first read, because the method depends on what was written; `--no-net` mirrors `--no-file`.
8. **HTTP in the browser** goes through `fetch`. `serviceRing` is now asynchronous: it awaits each host call before the next entry, then the doorbell is rung, which the ring design allowed from the start, since the worker waits on the doorbell whatever the main thread does.
9. **9p mounts** use a plain 9P2000 client over TCP (`crates/runtime/src/ninep.rs`): version, attach, walk, open, create, read, write, clunk and stat, msize 8192, one request in flight. A mount connects on its first open, so an unreachable server costs nothing until used. A directory read maps the server's stat records onto Chasm's directory records. The in-memory test server (`crates/runtime/tests/common/ninep_server.rs`) is shared with the CLI's test by `#[path]`. 9p from the browser, over WebSocket, is deferred.
10. **WASI export is a mapping of four words and a path table** (`crates/core/src/wasi.rs`), not an API translation. `build --wasi` imports five preview1 functions (`fd_write`, `fd_read`, `fd_close`, `path_open`, `clock_time_get`) in place of `chasm.ring_enter`, and the ring helper keeps its signature but dispatches over them, so compiled words are identical in both modes; the function index space shifts by the import count. `/dev/cons` is fds 0 and 1, `/dev/time` is `clock_time_get`, `/file/<path>` is `path_open` under preopen fd 3, anything else is not found, and directories are not readable. A trap writes its message to fd 2. It is verified by running built modules under `wasmtime-wasi` as a dev-dependency of the runtime, so no wasmtime CLI is needed.
11. **No io_uring backend.** Measured (`docs/performance.md`, Ring servicing): a trip through the ring costs about 120 to 200 ns, against 400 to 700 ns for the system call it carries. io_uring's gain is batching and overlapping submissions, but a Chasm host word submits one entry and waits for its completion before going on (the language is synchronous by design, section 5d), so with one entry in flight io_uring would only put a kernel round trip in place of a direct system call. Revisit if a host word ever submits several entries at once.

