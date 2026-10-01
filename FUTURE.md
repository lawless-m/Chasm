# Chasm: Future Directions

Status: draft v0.2. **Nothing in this document is v1.** It records how later features would fit the v1 design, so that v1 decisions do not close them off.

The pattern `str` sets, a checker-level type with a fixed documented lowering, carries every feature here.

## 1. Structs

By reference: a struct value is an `i32` pointer into linear memory. The user writes one declaration; the compiler generates the rest.

```
struct point  x: i32  y: f64
```

| Generated word | Effect | Lowering |
|---|---|---|
| `point.new` | `( i32 f64 -- point )` | Allocate, store fields. |
| `point.x` | `( point -- i32 )` | `i32.load offset=0` |
| `point.x!` | `( point i32 -- )` | `i32.store offset=0` |
| `point.y` / `point.y!` | `( point -- f64 )` / `( point f64 -- )` | `f64.load/store offset=8` |
| `point.size` | `( -- i32 )` | Constant. |

- Layout is natural alignment with padding; the compiler reports it on request. No hand-written offsets, ever.
- The `!` suffix means write, as for locals, so one idiom covers both.
- Generated words are ordinary dictionary words with effects: `words point` lists them, `forget point` removes them, and they appear in the dependency graph.
- Nested by-value fields (`origin: point`) are further loads at computed offsets.
- A by-value lowering (fields as separate wasm values, like `str`) may be offered as an attribute for small structs such as points and ranges.

Requires a real allocator with free, since structs are created and dropped constantly. The bump allocator is not enough.

## 2. Arrays and functions as values

Both moved into v1 (`LANGUAGE.md` 4a and 7a). Left for later: nested arrays (`array array i32`), arrays of structs, and a real allocator so `map` results can be freed.

## 3. Closures

A quotation plus captured locals: an environment struct and a funcref, with the function type gaining an environment parameter. Depends on by-reference structs and the allocator. Not before them.

## 4. The WasmGC alternative

Structs and arrays as native wasm GC types, with the engine's garbage collector. Removes the allocator problem entirely, and makes closures natural, at the cost of tying exported modules to GC-capable engines and changing the lowering story. Possibly the better path for structs and closures, with strings and arrays staying in linear memory. To be evaluated when section 1 becomes a goal.

## 5. Inference

Covered in `ARCHITECTURE.md` M6: an elaboration pass in front of the checker, row variables for the rest of the stack, and monomorphisation of type-variable effects. Shares machinery with first-class quotations.

## 6. Namespaces and libraries

A flat dictionary is fine until the library grows. Likely shape: a file is a module, `use name` imports it, words are prefixed by module (`str.len` is already this shape). Open question 2 in `LANGUAGE.md`.
