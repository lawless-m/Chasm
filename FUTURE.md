# Chasm: Future Directions

Status: draft v0.2. **Nothing in this document is v1.** It records how later features would fit the v1 design, so that v1 decisions do not close them off.

The pattern `str` sets, a checker-level type with a fixed documented lowering, carries every feature here.

## 1. Structs

Now milestone M4, as WasmGC structs rather than linear memory: see `ARCHITECTURE.md` section 15. Still for later: a by-value lowering (fields as separate wasm values, like `str`) for small structs such as points and ranges.

## 2. Arrays and functions as values

Both moved into v1 (`LANGUAGE.md` 4a and 7a). Left for later: nested arrays (`array array i32`) and a real allocator so `map` results can be freed. Arrays of structs are part of M4.

**Growable arrays.** Every array is a fixed-length view (`( addr len )`, or `( ref start len )` for an array of structs), and slicing shares storage. The growable array, `vec T`, is in the prelude since M9: a list of chunks whose sizes double (1, 2, 4, 8, ...), so growing allocates one new chunk as large as everything before it, at most 32 times, and storage never moves. It is library code written in Chasm (`struct chunk T  items: array T`, `struct vec T  chunks: array chunk T  count: i32`), beside the hash map `map K V` (`LANGUAGE.md` section 4d).
- Indexing is O(1): element `i` is in chunk `31 - clz(i + 1)` at offset `i + 1 - 2^chunk` (one `i32.clz` and a shift).
- Because nothing moves, every view into a chunk stays valid, so no separate owning-array and `slice` types are needed.

## 3. Closures

A quotation plus captured locals: an environment struct and a funcref, with the function type gaining an environment parameter. With structs as WasmGC structs (M4) the environment needs no allocator, so closures can follow structs directly.

## 4. WasmGC

Adopted for structs and unions (M4, M8). Strings and arrays of anything but structs stay in linear memory, allocated by a bump allocator that never frees: `[ "x" str.concat ] 1000000 times` keeps every string. A program that runs and exits does not notice; a long REPL session, a compiler, or a puzzle that builds and drops many collections would.

Collections built on today's arrays inherit this (vec and map are in the prelude since M9). A `vec` or `map` whose storage is an `array i32` or `array str` is never freed once dropped, so a loop that makes a fresh one per step (a search over a large grid) uses memory in proportion to the steps, not to what is live; reusing one collection (`vec.clear`) is the workaround. Fixing collections alone is half a fix while `str` still leaks, so the move is one decision for every non-struct value, a milestone of its own.

Moving `str` and the arrays to GC arrays (`array i8`, `array i32`, `array f64`, and a representation for `array str`, whose elements are two values) would put all memory under the collector, and the engine's own bounds checks on GC arrays would replace Chasm's. The costs:

- **I/O copies.** The ring hands the host an address and length in linear memory, so every read and write would copy between a GC array and a linear buffer.
- **Literals.** String literals live in a data segment; they would be built as GC arrays at start-up or with `array.new_data`.
- **Speed is unmeasured.** GC array access under Cranelift may be slower or faster than linear memory; `bench/` would decide.
- **The REPL stack.** Strings and arrays would join structs in the `chasm.refs` table, which already exists.
- **Views.** An `array T` is a view (`( addr len )`) that `array.slice` shares; over a GC array it becomes `( ref start len )`, as arrays of structs already are, so the rule carries over unchanged.
- **Raw memory.** `mem.alloc`, `i32.load` and the host words stay on linear memory, which remains for I/O buffers and low-level code.

## 5. Inference

Implemented in M7 (`ARCHITECTURE.md` section 19): effects may be left out and are inferred, and generic words with type variables are monomorphised per use. Still out of scope: row variables or stack-polymorphic effects in user syntax, constraints or type classes, and higher-rank types. Generic structs and unions came in M8.

## 6. Namespaces and libraries

A flat dictionary is fine until the library grows. Likely shape: a file is a module, `use name` imports it, words are prefixed by module (`str.len` is already this shape). Open question 2 in `LANGUAGE.md`.

## 7. Sum types

Implemented in M8. `union` declares a type with inline variants, each with its own fields, matched by postfix `match` with labelled arms and an optional `else:`; structs and unions take explicit type parameters and are monomorphised per instantiation, and the prelude's `option T` is the optional value. See `LANGUAGE.md` section 4c.

## 8. Self-hosting

A Chasm compiler written in Chasm, reached in stages, each an ordinary example with tests:

1. **Lexer.** Grow `examples/tokenizer.chasm` into a lexer whose tokens match `crates/core/src/lexer.rs` on the same input.
2. **Parser.** Items and bodies as unions (section 7): an AST node is a word, a literal or a quotation, matched exhaustively by `match`.
3. **Decide.** Measure what the first two stages cost before going further.

What a compiler needs that the language lacks today:

- **Growable arrays and maps.** Both are in the prelude since M9: `vec T` (section 2) and `map K V`.
- **Collected memory.** A compiler that runs and exits survives the bump allocator; one inside the REPL would want section 4's GC strings and arrays. An AST built from structs is collected already.

The end state is the usual fixpoint: the Rust compiler builds the Chasm compiler, that compiler builds itself, and the two outputs are byte-identical. The browser REPL could then run a compiler written in Chasm.

## 9. Concurrency: communicating processes

CSP-style processes and channels, built on closures (section 3). Chasm has no globals, so if a closure captures its locals by value, a process reaches only what it is given: a channel captured by a process is its whole connection to the rest of the program. The language enforces "share by communicating" rather than asking for it.

```
chan.make ( chan action ) :> ch
[ ch input tokenize-into ] spawn          # producer captures ch
[ ch chan.recv ... ] spawn                # consumer captures ch
```

Names follow Limbo (`spawn`, `alt`, channels); types are written the Chasm way, `chan T` like `vec T`.

- **Channels.** `chan T` is generic over `T`. `chan.send ( chan T T -- )` and `chan.recv ( chan T -- option T )` block the process, not the program.
- **Alt.** Labelled arms in the shape of `match`: `a recv: [ ... ] b recv: [ ... ] alt`.
- **Processes are green threads in one instance.** WasmGC references cannot cross threads until shared-everything threads ship, so a channel carrying a struct could not join two wasm threads.
- **Switching.** Either the Wasm stack-switching proposal (continuations; check engine and browser support when the time comes) or Binaryen's Asyncify, which works in every engine today at a cost in code size and speed. Binaryen is already a dependency (M5).
- **I/O.** The ring (`ARCHITECTURE.md` section 5d) already carries a `user` field on every entry. A scheduler can park a process on its pending completion and run another, so I/O becomes concurrent without changing the `host.*` words.
- **Stopping.** Limbo has no close: programs send a sentinel (`nil`), keep a separate quit channel, or kill the process group through `/prog`, and with two senders the sentinels have to be counted by hand. Chasm follows Rust's `mpsc`: a channel counts its senders and closes itself when the last one is done.
  - `chan.make` starts with one sender. `chan.sender ( chan T -- )` adds one, before the `spawn` that captures it.
  - `chan.close` says "this sender is done". The channel closes when every sender has closed.
  - `chan.recv` returns `option T`: `some` while values remain, `none` once the channel is closed and drained, matched like `map.get`.
  - Sending on a closed channel traps, and so does closing it once more than it has senders.
  - A forgotten `chan.close` leaves the reader waiting. When every process is blocked on a channel and none is waiting on the ring, the scheduler traps (`all processes blocked`, naming each process and the channel it waits on) rather than hanging. A process waiting on I/O is not blocked: its completion will wake it.
  - `mpsc` closes when the last sender is dropped. Chasm has no drop (structs are collected), so the count is explicit.
  - Killing a process could be a write to a namespace entry, as Inferno's `/prog/<pid>/ctl`, not a new word.

```
chan.make ( chan action ) :> ch
ch chan.sender                                        # two senders
[ ch file1 tokenize-into  ch chan.close ] spawn
[ ch file2 tokenize-into  ch chan.close ] spawn
[ [ 1 ] [ ch chan.recv  none: [ leave ] some: [ handle ] match ] while ] spawn
```

Open questions:

- **One channel or two ends.** To be settled before any channel code is written, because a program written for one shape has to be rewritten for the other. It depends on how closures capture (section 3), so it waits for that design.
  - *Single `chan T`*, as described above: one value, used for both sending and receiving, with an explicit sender count. This is Limbo's shape, and a process may send and receive on the same channel. A reader that stops early leaves an infinite producer blocked on `chan.send`, because every holder of the channel might still be a reader.
  - *Two typed ends*, after the pipe: `chan.make ( -- tx T rx T )`, bound as `chan.make ( action ) :> rx :> tx`, with `chan.send ( tx T T -- )` and `chan.recv ( rx T -- option T )`. Both ends are counted. A closure that captures an end is counted at `spawn`, read off the captured type, and a process's ends are released when it exits, so `chan.sender` is not needed. When the last sender is gone `chan.recv` gives `none`; when the last reader is gone the sender ends quietly at its next `chan.send`, as a writer does on a closed pipe. `chan.close` remains for letting go early: a parent that keeps its `tx` after spawning the senders must close it or the reader never sees `none`.
  - With two ends, ends are captured and not sent: a word that makes the channel, spawns a process holding one end and returns the other (a generator, a pipeline stage) takes the place of a channel of channels. Two running processes cannot then be introduced to each other, so a reply to a client needs either a call word (request and reply as one rendezvous) or ends allowed as messages with ownership moving to the receiver. Both can be added later without breaking programs.
  - *Both, under different names.* With the two ends as the primitive, the single channel is a prelude struct holding a `tx T` and an `rx T`, with a word to pick out either end. A process that captures the pair counts as a sender and a reader, so such a channel never stops by itself while a holder lives, which is Limbo's behaviour; handing a process one end gives it the pipe's. The pair can be added after the two ends without breaking programs, so the order is two ends first. The reverse does not work: typed ends added over a single counted `chan T` are only views, and the runtime still cannot tell a holder that reads from one that only writes. A producer that captures the pair where a `tx T` would do loses the automatic stop; its declared effect shows `chan T`. The pair puts ends in struct fields, so the checker sees through fields when it counts a capture. That cost is paid once per `spawn`, not per message.
- **Sharing.** A struct, `vec` or `map` sent on a channel is a reference, so sender and receiver share it. The choices are to accept that (as Limbo and Go do), to copy on send, or to allow only immutable values (numbers, `str`, unions) on channels.
