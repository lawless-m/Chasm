# Chasm: Future Directions

Status: draft v0.2. **Nothing in this document is v1.** It records how later features would fit the v1 design, so that v1 decisions do not close them off.

The pattern `str` sets, a checker-level type with a fixed documented lowering, carries every feature here.

## 1. Structs

Now milestone M4, as WasmGC structs rather than linear memory: see `ARCHITECTURE.md` section 15. Still for later: a by-value lowering (fields as separate wasm values, like `str`) for small structs such as points and ranges.

## 2. Arrays and functions as values

Both moved into v1 (`LANGUAGE.md` 4a and 7a). Left for later: nested arrays (`array array i32`) and a real allocator so `map` results can be freed. Arrays of structs are part of M4.

**Growable arrays.** Every v1 array is a fixed-length view (`( addr len )`, or `( ref start len )` for an array of structs), and slicing shares storage. The preferred growable array never moves its storage: it is a list of chunks whose sizes double (1, 2, 4, 8, ...), so growing allocates one new chunk as large as everything before it, at most 32 times.
- Indexing is O(1): element `i` is in chunk `31 - clz(i + 1)` at offset `i + 1 - 2^chunk` (one `i32.clz` and a shift).
- Because nothing moves, every view into a chunk stays valid, so no separate owning-array and `slice` types are needed.
- A view is still contiguous, so it cannot span two chunks; walking the whole array goes through `each`/`fold`-style words on the growable array itself.
- With M4 structs it is library code, not a language feature, and needs no nested arrays: `struct chunk  items: array i32` and `struct vec-i32  chunks: array chunk  count: i32`. Struct fields cannot be type variables, so it is still written once per element type (`vec-i32`, `vec-str`, `vec-point`); generic words share only the code that touches no struct.

## 3. Closures

A quotation plus captured locals: an environment struct and a funcref, with the function type gaining an environment parameter. With structs as WasmGC structs (M4) the environment needs no allocator, so closures can follow structs directly.

## 4. WasmGC

Adopted for structs in M4 (`ARCHITECTURE.md` section 15). Strings and arrays of numbers stay in linear memory, allocated by a bump allocator that never frees: `[ "x" str.concat ] 1000000 times` keeps every string. A program that runs and exits does not notice; a long REPL session or a compiler would.

Moving `str` and numeric arrays to GC arrays (`array i8`, `array i32`, ...) would put all memory under the collector, and the engine's own bounds checks on GC arrays would replace Chasm's. The costs:

- **I/O copies.** The ring hands the host an address and length in linear memory, so every read and write would copy between a GC array and a linear buffer.
- **Literals.** String literals live in a data segment; they would be built as GC arrays at start-up or with `array.new_data`.
- **Speed is unmeasured.** GC array access under Cranelift may be slower or faster than linear memory; `bench/` would decide.
- **The REPL stack.** Strings and arrays would join structs in the `chasm.refs` table, which already exists.

## 5. Inference

Implemented in M7 (`ARCHITECTURE.md` section 19): effects may be left out and are inferred, and generic words with type variables are monomorphised per use. Still out of scope: row variables or stack-polymorphic effects in user syntax, constraints or type classes, higher-rank types, and generic structs (struct fields cannot be type variables).

## 6. Namespaces and libraries

A flat dictionary is fine until the library grows. Likely shape: a file is a module, `use name` imports it, words are prefixed by module (`str.len` is already this shape). Open question 2 in `LANGUAGE.md`.

## 7. Sum types

A type that is one of several shapes: `list = nil | cons`, a token kind, a JSON value. Without them, M4 code writes an optional link as an `array` of length 0 or 1 (`next: array node`, tested with `array.len`), the way Prolog's `[]` ends a list but as an empty container rather than its own value. Sum types would retire that idiom.

With WasmGC the lowering is natural: one non-final supertype per sum type, a final struct subtype per variant, and matching by `br_on_cast` / `ref.test`. The checker would need an exhaustive match combinator (one quotation per variant, all leaving the same stack), in the style of `if`. Depends on M4 structs.

## 8. Self-hosting

A Chasm compiler written in Chasm, reached in stages, each an ordinary example with tests:

1. **Lexer.** Grow `examples/tokenizer.chasm` into a lexer whose tokens match `crates/core/src/lexer.rs` on the same input.
2. **Parser.** Items and bodies as structs. This is where the lack of sum types (section 7) bites: an AST node is a word, a literal or a quotation, and without sum types each becomes a tagged struct with fields used only sometimes, matched by `if` chains the checker cannot prove exhaustive.
3. **Decide.** Measure what the first two stages cost, then choose whether sum types and generic structs (generic words exist since M7) come first.

What a compiler needs that the language lacks today:

- **Generic collections.** Generic words cover helpers over `array T`, but structs are not generic, so a growable array, a token list or a symbol table built from structs is still written once per element type.
- **Growable arrays and maps.** The chunked growable list (section 2) and a hash map, again per type.
- **Collected memory.** A compiler that runs and exits survives the bump allocator; one inside the REPL would want section 4's GC strings and arrays. An AST built from structs is collected already.

The end state is the usual fixpoint: the Rust compiler builds the Chasm compiler, that compiler builds itself, and the two outputs are byte-identical. The browser REPL could then run a compiler written in Chasm.
