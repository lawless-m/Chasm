# Whackford: Future Directions

Status: draft v0.5. **Nothing in this document is v1.** It records how later features would fit the v1 design, so that v1 decisions do not close them off.

The pattern `str` sets, a checker-level type with a fixed documented lowering, carries every feature here.

## 1. Structs

Now milestone M4, as WasmGC structs rather than linear memory: see `ARCHITECTURE.md` section 15. Still for later: a by-value lowering (fields as separate wasm values, like `str`) for small structs such as points and ranges.

## 2. Arrays and functions as values

Both moved into v1 (`LANGUAGE.md` 4a and 7a). Left for later: nested arrays (`array array i32`) and a real allocator so `map` results can be freed. Arrays of structs are part of M4.

**Growable arrays.** Every array is a fixed-length view (`( addr len )`, or `( ref start len )` for an array of structs), and slicing shares storage. The growable array, `vec T`, is in the prelude since M9: a list of chunks whose sizes double (1, 2, 4, 8, ...), so growing allocates one new chunk as large as everything before it, at most 32 times, and storage never moves. It is library code written in Whackford (`struct chunk T  items: array T`, `struct vec T  chunks: array chunk T  count: i32`), beside the hash map `map K V` (`LANGUAGE.md` section 4d).
- Indexing is O(1): element `i` is in chunk `31 - clz(i + 1)` at offset `i + 1 - 2^chunk` (one `i32.clz` and a shift).
- Because nothing moves, every view into a chunk stays valid, so no separate owning-array and `slice` types are needed.

## 3. Closures

Implemented in M10 (`ARCHITECTURE.md` section 23): a function value is a closure, a WasmGC struct holding the slot of its code and the immutable locals it captured by value; quotation values take inputs. See `LANGUAGE.md` section 7a.

## 4. WasmGC

Adopted for structs and unions (M4, M8). Strings and arrays of anything but structs stay in linear memory, allocated by a bump allocator that never frees: `[ "x" str.concat ] 1000000 times` keeps every string. A program that runs and exits does not notice; a long REPL session, a compiler, or a puzzle that builds and drops many collections would.

Collections built on today's arrays inherit this (vec and map are in the prelude since M9). A `vec` or `map` whose storage is an `array i32` or `array str` is never freed once dropped, so a loop that makes a fresh one per step (a search over a large grid) uses memory in proportion to the steps, not to what is live; reusing one collection (`vec.clear`) is the workaround. Fixing collections alone is half a fix while `str` still leaks, so the move is one decision for every non-struct value, a milestone of its own.

Moving `str` and the arrays to GC arrays (`array i8`, `array i32`, `array f64`, and a representation for `array str`, whose elements are two values) would put all memory under the collector, and the engine's own bounds checks on GC arrays would replace Whackford's. The costs:

- **I/O copies.** The ring hands the host an address and length in linear memory, so every read and write would copy between a GC array and a linear buffer.
- **Literals.** String literals live in a data segment; they would be built as GC arrays at start-up or with `array.new_data`.
- **Speed is unmeasured.** GC array access under Cranelift may be slower or faster than linear memory; `bench/` would decide.
- **The REPL stack.** Strings and arrays would join structs in the `wack.refs` table, which already exists.
- **Views.** An `array T` is a view (`( addr len )`) that `array.slice` shares; over a GC array it becomes `( ref start len )`, as arrays of structs already are, so the rule carries over unchanged.
- **Raw memory.** `mem.alloc`, `i32.load` and the host words stay on linear memory, which remains for I/O buffers and low-level code.

## 5. Inference

Implemented in M7 (`ARCHITECTURE.md` section 19): effects may be left out and are inferred, and generic words with type variables are monomorphised per use. Still out of scope: row variables or stack-polymorphic effects in user syntax, constraints or type classes, and higher-rank types. Generic structs and unions came in M8.

## 6. Namespaces and libraries

A flat dictionary is fine until the library grows. Likely shape: a file is a module, `use name` imports it, words are prefixed by module (`str.len` is already this shape). Open question 2 in `LANGUAGE.md`.

## 7. Sum types

Implemented in M8. `union` declares a type with inline variants, each with its own fields, matched by postfix `match` with labelled arms and an optional `else:`; structs and unions take explicit type parameters and are monomorphised per instantiation, and the prelude's `option T` is the optional value. See `LANGUAGE.md` section 4c.

## 8. Self-hosting

A Whackford compiler written in Whackford, reached in stages, each an ordinary example with tests:

1. **Lexer.** Grow `examples/tokenizer.wack` into a lexer whose tokens match `crates/core/src/lexer.rs` on the same input.
2. **Parser.** Items and bodies as unions (section 7): an AST node is a word, a literal or a quotation, matched exhaustively by `match`.
3. **Decide.** Measure what the first two stages cost before going further.

What a compiler needs that the language lacks today:

- **Growable arrays and maps.** Both are in the prelude since M9: `vec T` (section 2) and `map K V`.
- **Collected memory.** A compiler that runs and exits survives the bump allocator; one inside the REPL would want section 4's GC strings and arrays. An AST built from structs is collected already.

The end state is the usual fixpoint: the Rust compiler builds the Whackford compiler, that compiler builds itself, and the two outputs are byte-identical. The browser REPL could then run a compiler written in Whackford.

## 9. Concurrency: communicating processes

Implemented: `chan T` with the explicit counted close below, `chan.recv` giving `option T`, `spawn`, `alt`, the blocked trap and `/prog/<pid>/ctl` kill (`LANGUAGE.md` section 4f, `ARCHITECTURE.md` sections 5g and 24), in the browser REPL and natively, where the compiler unwinds a waiting process's stack into GC frames and rewinds it on resume (`ARCHITECTURE.md` sections 5g, 25 and 26; first in the browser in M11, natively in M12, one mechanism since M13).

CSP-style processes and channels, built on closures (section 3). Whackford has no globals and closures capture their locals by value, so a process reaches only what it is given: a channel captured by a process is its whole connection to the rest of the program. The language enforces "share by communicating" rather than asking for it.

```
chan.make ( chan action ) :> ch
[ ch input tokenize-into ] spawn          # producer captures ch
[ ch chan.recv ... ] spawn                # consumer captures ch
```

Names follow Limbo (`spawn`, `alt`, channels); types are written the Whackford way, `chan T` like `vec T`.

- **Channels.** `chan T` is generic over `T`, one value used for both sending and receiving, as in Limbo. `chan.send ( chan T T -- )` and `chan.recv ( chan T -- option T )` block the process, not the program. A send is a rendezvous: it waits until a receiver takes the value.
- **Spawn.** `spawn ( [ -- ] -- )` is an ordinary word taking a function value, like `vec.each`: `[ ... ] spawn`, `f spawn` and `'worker spawn` all work. A process is a closure; what it captured is all it can reach.
- **Alt.** Labelled arms in the shape of `match`: `a recv: [ ... ] b recv: [ ... ] alt`.
- **Processes are green threads in one instance.** WasmGC references cannot cross threads until shared-everything threads ship, so a channel carrying a struct could not join two wasm threads. Switching is cooperative, at channel operations and the end of a process only.
- **Switching.** Every switch is a ring entry. In every host, `ring_enter` returns at once with the mode cell set to unwinding: the compiler's transform saves every frame on the stack into GC structs, and rewinds them when the process resumes. Binaryen's Asyncify is not an option: it does not support reference-typed locals, which every process has (its closure).
- **I/O.** I/O completes inline in every host: a process doing I/O keeps running, and no process ever waits on the ring.

**Stopping is out of band.** A reader learns that a channel is finished from the channel, never from a value in the data: no sentinel (Limbo's `nil`) and no reserved value, which would mix control into data and, with several senders, need counting by hand in every reader. The channel counts its senders explicitly and closes when the last one says it is done:

- `chan.make` starts with one sender. `chan.sender ( chan T -- )` adds one, before the `spawn` that captures it.
- `chan.close` says "this sender is done". The channel closes when every sender has closed.
- `chan.recv` returns `option T`: `some` while values remain, `none` once the channel is closed and drained, matched like `map.get`.
- Sending on a closed channel traps, and so does closing it once more than it has senders.
- A forgotten `chan.close` leaves the reader waiting. When nothing is ready and some process waits on a channel, the scheduler traps (`all processes blocked`, naming each process and the channel it waits on) rather than hanging. This detector is what makes an explicit close safe to rely on.
- Killing a process is a write of `kill` to a namespace entry, `/prog/<pid>/ctl`, as in Inferno, not a new word.

The close is explicit, not automatic. Rust's `mpsc` closes when the last sender is dropped, but Whackford has no drop: WasmGC collects values without telling the program, so knowing that the last holder is gone would mean counting holders by hand at every capture, `spawn` and exit, including through closures captured by closures and ends kept in mutable struct fields. That is a large mechanism for the one convenience of not writing `chan.close`. An automatic close can be added later on top of the explicit one without breaking programs that close explicitly, so it waits until programs show the need.

```
chan.make ( chan action ) :> ch
ch chan.sender                                        # two senders
[ ch file1 tokenize-into  ch chan.close ] spawn
[ ch file2 tokenize-into  ch chan.close ] spawn
[ [ 1 ] [ ch chan.recv  none: [ leave ] some: [ handle ] match ] while ] spawn
```

What this leaves to the program, as Limbo and Go do: a producer whose reader stops early blocks on its next `chan.send` (the detector reports it if nothing else can run), and with several senders each must close exactly once.

Open questions:

- **Typed ends.** `chan.make` could give a sending end and a receiving end (`tx T`, `rx T`), so a word's effect shows which way it uses a channel and the checker refuses a receive on a sending end. Over the explicit close these are views of one channel, checked statically, and can be added after `chan T` without breaking programs.
- **Automatic close**, above: only if programs show the need.
- **`send:` arms in `alt`**, to wait on being able to send as well as receive.

Sharing was decided in M11: a struct, `vec` or `map` sent on a channel or captured by two processes is shared, as in Limbo and Go. Switching happens only at channel operations and the end of a process, so sharing is interleaving at known points rather than a data race.

## 10. Visibility: values that cannot be invalid

A struct groups values but guarantees nothing about them: its generated `s.new` and `s.f!` are callable from anywhere, so `-5 0 dims.new` builds a `dims` that no image has. Programs check at their boundary and trust the values afterwards (the JPEG decoder refuses in its marker parser), which is a convention the checker does not know.

Part of the fix exists. A generated word is an ordinary dictionary word, so a same-effect redefinition replaces it for every caller, and inside the new body the name still reaches the generated word:

```wack fragment
struct dims  w: i32  h: i32
: dims.new ( i32 i32 -- dims )
  :> h :> w
  w 1 i32.lt_s [ "bad width" trap ] when
  w h dims.new ;
```

Every `dims` then passed the check, as Haskell's smart constructors give: export the type and a checking constructor, hide the raw one. Two gaps remain:

- The `!` writers make a valid value invalid afterwards (`d -5 dims.w!`), and cannot be withheld.
- That the redefined body reaches the generated word is what the compiler does, not a documented rule. A pattern relied on for safety needs the rule written and a test.

The general form is one rule, not a list of special constructors (`range`, `nonempty`, ...): a struct's `.new` and `!` writers callable only from a named scope (a module, section 6), as a Haskell export list hides a data constructor. With it, holding a `dims` proves it was checked. Unsigned or range-restricted integer types are the narrower alternative; they would cover sign and range but nothing else (a length matching another field, a non-empty array), so the scoped constructor comes first. This is a checker rule and needs nothing else on this page.

## 11. Compile-time words

Words that run in the compiler, as Forth's immediate and defining words and Lisp's macros do. `struct` is already a defining word built into the compiler; compile-time words would let a library define its own.

Two uses the JPEG codec shows:

1. **Constant tables.** There is no array literal, so tables are hex strings parsed at run time, and each table word carries the rule "allocates, so callers build it once, outside any loop". A pure word with no inputs, evaluated once by the compiler, would become constant data in the module.
2. **Checked constructors** (section 10) as library words: `range dim 1 65535` defining a type and its refusing constructor, written in Whackford, not added to the checker.

Quotations are already code as data; Factor, the nearest relative, builds its macros on words that take and return quotations at compile time.

The cost is that the compiler must run Whackford. With the compiler in Rust that means either an interpreter in `crates/core`, a second semantics to keep identical to the compiled one (wrapping, traps, the GC types), or the host compiling and running words during compilation, which breaks core's rule of no I/O and makes every host part of compiling. A self-hosted compiler (section 8) removes the problem: it is itself a Whackford program, so a compile-time word is a word it calls, with one semantics. Hence the order:

1. Pure, input-less words evaluated at compile time (the constant tables), possibly on the REPL's compile-then-call machinery before self-hosting. The checker already knows which words qualify.
2. Defining words, once self-hosting is past section 8's "decide" stage.

Generated words must stay visible to the tools: `deps`, `dead`, `)forget` and the language server already handle the words `struct` and `union` generate, and user-written generators make that open-ended. Factor's parsing words show the cost of leaving it to each generator, which is why the first stage is the constant case.
