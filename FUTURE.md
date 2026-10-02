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
- With M4 structs it is library code, not a language feature, and needs no nested arrays: `struct chunk  items: array i32` and `struct vec-i32  chunks: array chunk  count: i32`. Without generics it is written once per element type (`vec-i32`, `vec-str`, `vec-point`).

## 3. Closures

A quotation plus captured locals: an environment struct and a funcref, with the function type gaining an environment parameter. With structs as WasmGC structs (M4) the environment needs no allocator, so closures can follow structs directly.

## 4. WasmGC

Adopted for structs in M4 (`ARCHITECTURE.md` section 15). Strings and arrays stay in linear memory; whether arrays should become GC arrays too is open.

## 5. Inference

Covered in `ARCHITECTURE.md` M7: an elaboration pass in front of the checker, row variables for the rest of the stack, and monomorphisation of type-variable effects. Shares machinery with first-class quotations.

## 6. Namespaces and libraries

A flat dictionary is fine until the library grows. Likely shape: a file is a module, `use name` imports it, words are prefixed by module (`str.len` is already this shape). Open question 2 in `LANGUAGE.md`.

## 6. Sum types

A type that is one of several shapes: `list = nil | cons`, a token kind, a JSON value. Without them, M4 code writes an optional link as an `array` of length 0 or 1 (`next: array node`, tested with `array.len`), the way Prolog's `[]` ends a list but as an empty container rather than its own value. Sum types would retire that idiom.

With WasmGC the lowering is natural: one non-final supertype per sum type, a final struct subtype per variant, and matching by `br_on_cast` / `ref.test`. The checker would need an exhaustive match combinator (one quotation per variant, all leaving the same stack), in the style of `if`. Depends on M4 structs.
