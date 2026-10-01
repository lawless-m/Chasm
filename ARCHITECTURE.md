# Chasm: Architecture and Milestones

Status: draft v0.9 (M0 and M1 implemented; decisions in section 13). **Chasm** (Chuck-Wasm, after Chuck Moore) is a typed, concatenative language that compiles to WebAssembly, with an interactive REPL, written in Rust. Source files use the `.chasm` extension; the CLI binary is `chasm`.

## 1. Goals

- A small Forth/Factor-like concatenative language whose words map almost one-to-one onto wasm.
- Every word carries a **declared stack effect**. The checker is the single source of truth.
- An **interactive workflow**: type words, see them run, redefine them live.
- **Contract first**: declare a word's effect (and optional tests) before writing its body.
- The compiler core runs **both natively and as wasm in the browser** from the same source.
- Designed so that **Claude Code can write and fix programs** from compiler feedback alone.

Non-goals for v1: inference, structs, closures, garbage-collected heap types (WasmGC), the Component Model, multi-threading. See `FUTURE.md` for how these would fit. Arrays and functions as values (without capture) **are** v1.

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
| `cli` | `check`, `build`, `run`, `deps`, `used-by`, `unresolved`, `test` | Text and JSON output on every command. |
| `web` | Browser REPL; loads `core` as wasm | Static files only, no server. |
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
- **Per-word modules in the REPL.** Each new or redefined word compiles to a small module that imports the shared table and memory. Compile cost is expected to be small enough for instant feedback; measure early (see milestone M2).
- **Memory layout.** One linear memory, fixed regions at fixed offsets, agreed by the REPL, host and exporter:

  | Offset | Region | Notes |
  |---|---|---|
  | 0 to 64 KiB | Reserved | Address 0 is never valid; loads and stores here are a bug. |
  | 64 KiB to 128 KiB | I/O ring | Submission and completion queues (section 5d). |
  | 128 KiB to 1 MiB | Data stack | Fixed size. Overflow traps with a message. |
  | 1 MiB upward | Literals, then heap | Read-only literals first, bump heap after, growing by `memory.grow`. |

  Initial memory is 4 MiB (64 pages). These numbers are v1 defaults, not promises; they live in one place in `core`.
- **Memory ownership.** v1 uses a bump allocator with no free. Strings and other non-numeric data (open question 4) build on it. A real allocator is a later concern and must not leak into the language design.

## 5a. Locals

Wasm provides locals for free, and the language uses them at two levels:

- **Named locals as sugar.** A word may bind values off the stack into word-scoped names with Factor's `:> name` (`:> name!` for mutable). Each local is typed by whatever was on the stack when it was bound, so the checker needs no new theory, and the emitter maps them directly to `local.get`/`local.set`. The word's effect is unchanged, so the interface story is untouched. This is the single biggest aid to writing correct code, human or machine: bind the inputs, and the stack only matters at the edges.
- **Compiler-only locals.** The emitter uses wasm locals internally to implement shuffles such as `rot` and `pick` that have no cheap operand-stack encoding. Users never see these.

v1 limits: locals are word-scoped and not captured by quotations (that would mean closures). Mutation is explicit via the `!` suffix. Shuffle words (`dup`, `swap`, `over`, `drop`) remain idiomatic for short words; locals are the expected choice beyond three or so values.

## 5b. Polymorphism

Wasm has no polymorphic instructions, and a word's effect is its wasm function type, so **user words are monomorphic**. The only polymorphism in the language is built into the shuffle primitives.

- **Shuffle primitives** (`dup`, `swap`, `drop`, `over`, `rot`, `nip`, `tuck`, `2dup`) and **collection combinators** (`each`, `map`, `filter`, `fold`) have effects written with type variables, for example `( a b -- b a )`. Checking is forward and every user word is concretely typed, so the stack is always concrete when the checker meets one of these. It instantiates the primitive at the types present and the emitter produces the matching code: `dup` on `i32` is `local.tee` into a compiler-only `i32` local, `swap` is two locals, and so on.
- **No type variables in user effects.** A word that would need `( a b -- a b a b )` must instead be written once per type (`2dup-i32`, `2dup-f64`). Tedium is chosen over magic: every user word has a real wasm function type, a real table slot, and a single node in the dependency graph.
- **Later, if wanted:** monomorphisation, where user effects may carry type variables and each call site gets its own specialised instance. That needs per-instance table slots and graph nodes, and shares machinery with M6 inference, so it is deferred to that point and may never be needed.

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
| `/net/http/<host>/<path>` | Opens an HTTP request. Write sends a body, read receives the response. |
| `/file/...` | Host filesystem, where one exists. |
| `/mnt/<name>/...` | Mounted trees, including 9p servers. |

**Directories.** Reading a directory handle returns entries in one fixed encoding: a length-prefixed record per entry (name, size, is-dir flag). It is the same everywhere, so a directory-walking word works on local files, the browser's in-memory tree, or a remote 9p mount alike.

**Mounts.** Host configuration in v1: the CLI takes `--mount /mnt/name=<source>`, where a source may be a local directory or a 9p server (`9p://host:564`, or over WebSocket in the browser). A `host.mount` word may come later.

**Transport: a submission/completion ring from day one.** Each `host.*` word writes a request into a ring in linear memory, waits for the completion, and returns the result. The words look synchronous to the language; asynchrony lives entirely in the host.

- **Native host**: services the ring with files, an HTTP client, a 9p client, and optionally real io_uring as the backend.
- **Browser host**: the wasm instance runs in a Web Worker. The main thread services the ring with `fetch`, WebSocket (for 9p) and the DOM; the worker blocks on `Atomics.wait`. This is what makes blocking reads possible in the browser at all.

**What this buys:**
- `check` can list the paths a word opens, so a host lacking `/file` can say so before running.
- Tests can mount a fake namespace: `/dev/cons` captured into a buffer, `/net/http/...` answered from a fixture.
- WASI export is a mapping of four words and a path table, not an API translation.
- One I/O idiom for Claude Code to learn, and it is the Unix one.

**Cost:** the runtime layer is built properly in M1 rather than later, and browser `run` mode depends on the worker in M2. Error codes and the directory record encoding are specified in `LANGUAGE.md`.

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

Later (not v1): an explicit force command that cascades an effect change through dependants, using the dependency graph for rebuild order. Policy for broken dependants (error, mark broken, keep old version) to be decided then.

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
- Inlining candidates (leaf words) at export.
- Cycle detection via strongly connected components. In v1 every word is declared, so this matters only for M6 inference, where recursive and mutually recursive words must keep declared effects.
- Incremental checking after a body edit.
- `deps`, `used-by` and `unresolved` commands with JSON output for tooling and Claude Code.

## 8. Export (hybrid mode)

The REPL uses indirect calls. The export step does whole-program compilation: direct calls, inlining of leaf and small words, dead-word removal, then optional Binaryen optimisation via an external `wasm-opt` binary (not a crate dependency, so the browser build stays light). It fails if any reachable declared word is unresolved. Same source, two back ends; the checker is shared.

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

**M4: Hybrid export.** Whole-program compilation, direct calls, inlining, dead-word removal, Binaryen. Refuse unresolved words.

**M5: Polish and tooling.** JSON output everywhere, editor integration, expanded examples, force-redefine with cascade. `/net/http`, 9p mounts, WASI export mapping, io_uring native backend if profiling justifies it.

**M6: Inference (optional).** Elaboration pass in front of the checker producing annotations, with row variables. Soundness unaffected because the checker re-verifies. Start with straight-line words made only of primitives, then add unification.

M1 to M3 can overlap; the graph and stub data structures are part of M1 so that M3 is tooling only.

## 11. Open questions

Settled questions live in `LANGUAGE.md` (primitive set, numeric types, strings, effect syntax, tests; M1 decisions in its section 12). Still open:

1. Error-recovery policy for broken dependants when the cascade (force-redefine) mode is added in M5.
2. Load/store alignment and offset immediates (`LANGUAGE.md` open item 1): v1 is natural alignment, offset 0.
3. Module or namespace structure for libraries (`LANGUAGE.md` open item 2); a flat dictionary until it hurts.
4. `/net/http` semantics (methods, headers) when it is built in M5.

Row variables in effects are an M6 matter, not a v1 question; the type representation leaves room for them (section 4).

## 12. Hardware notes

Heavy native batch work (large test corpora, benchmarks, Binaryen runs over big programs) can use the work machine (3090, dual Xeon, 64 GB). Day-to-day development runs comfortably on the home machine (4070, 16 GB, Debian). Nothing in the design needs a GPU.

## 13. Decisions taken in M0 and M1

1. **Crates.** `crates/core` (package `chasm-core`), `crates/runtime` (`chasm-runtime`, wasmtime behind the default `native` feature), `crates/cli` (`chasm-cli`, binary `chasm`). `web/` is a placeholder until M2. `core` builds for `wasm32-unknown-unknown`, and CI checks that.
2. **One doorbell import.** The compiled module imports exactly one host function, `chasm.ring_enter ( -- )`. The four `host.*` words are compiled code: each writes a submission entry into the ring, calls the doorbell, and takes one completion. The native host services the ring synchronously inside the doorbell; the browser host will post to the main thread and `Atomics.wait`. Ring layout (in `core::layout`): heads and tails at 64 KiB; 256 submission entries of 32 bytes (`op, user, a0, a1, a2`); 256 completion entries of 16 bytes (`user, result`). Opcodes: 1 open, 2 read, 3 write, 4 close.
3. **Runtime cells** in the reserved region: trap message address and length at `0x100`/`0x104`, trapping word at `0x108`/`0x10C`, heap pointer at `0x110`, data stack pointer (M2) at `0x114`. Address 0 stays invalid.
4. **Trap messages.** `trap`, bounds checks, unresolved stubs and out-of-memory call a runtime helper that writes the message and word into those cells, then executes `unreachable`. The host reads them back. Plain wasm traps (division by zero) are named from the module's name section via the backtrace.
5. **M1 module shape.** One module per program: runtime helpers (`rt.alloc`, `rt.trap`, `rt.ring`), then one function per word in definition order. Calls are direct. Every word also owns a slot in a funcref table (slot = word index) for `'word` and `call` (`call_indirect`). The module defines and exports its memory; the M2 REPL will import it instead. Literals start at 1 MiB, and the heap follows them.
6. **Functions as values** (`'word`, quotation values, `call`, address-taken edges) were cheap on top of the table, so they landed in M1 rather than M3.
7. **Two-pass checking.** Each body is walked twice by the same checker: a checking pass that settles type variables, then an emitting pass with the final substitution. Blocks take the whole checker stack as parameters (multi-value), so a quotation under a combinator can reach any value below it.
8. **Text from JSON.** Every CLI command builds the JSON report; the text output is rendered from that JSON value.
9. **Host namespace.** `/file/<path>` is the host path `/<path>` (off with `--no-file`). `--mount name=DIR` mounts a local directory at `/mnt/name`, and `..` is refused under mounts. `/net/...` and 9p sources return "not supported" until M5.
