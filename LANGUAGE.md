# Chasm: Language Specification

Status: draft v0.15 (M1 to M9 decisions recorded; see sections 12 to 20). Chasm source files use the `.chasm` extension. Companion to `ARCHITECTURE.md`. Sections marked **TBD** are not yet decided.

## 1. Types

| Type | Wasm lowering | Notes |
|---|---|---|
| `i32` | `i32` | Also used for booleans (0 false, non-zero true) and memory addresses (wasm32). |
| `i64` | `i64` | |
| `f32` | `f32` | |
| `f64` | `f64` | |
| `str` | `i32 i32` (addr, byte length) | Checker-level type with a fixed, documented lowering. See section 4. |
| `bytes` | `i32 i32` (addr, byte length) | A mutable byte buffer that carries its length. See section 4e. |
| `array T` | `i32 i32` (addr, element count); for a struct `T`, `(ref null $T_array) i32 i32` (GC array, start, count) | `T` is any type in this table except `array`. See sections 4a and 4b. |
| struct name | `(ref null $name)` | A declared struct: a reference to a WasmGC struct. See section 4b. |
| union name | `(ref null $name)` | A declared union: a reference to one of its variants, WasmGC subtypes of the union's type. See section 4c. |
| `name T...` | `(ref null $instance)` | A generic struct or union applied to type arguments (`pair i32 str`): its own WasmGC type per instantiation. See section 4c. |
| `[ effect ]` | `i32` (function table index) | A quotation type: a function as a value. See section 7a. |

Signedness is a property of operations, not types, as in wasm. There is no `bool`, `char`, `u32`, `v128` or nested array in v1.

A word's effect lists checker types. Its wasm function type is the effect with each type lowered, in order. So `( str i32 -- str )` has wasm type `(i32 i32 i32) -> (i32 i32)`.

## 2. Literals

| Form | Type | Example |
|---|---|---|
| decimal or hex integer | `i32` | `42`, `-7`, `0xFF` |
| integer followed by the word `i64` | `i64` | `42 i64`, `0xFF i64` |
| decimal with a point or exponent | `f64` | `1.5`, `2e10` |
| double-quoted | `str` | `"hello"` |

String literals are UTF-8, immutable, and live in read-only data. Escapes: `\"`, `\\`, `\n`, `\t`, `\u{XXXX}`. Integer literals that do not fit their type are a compile error; an `i32` literal may be anything from -2^31 to 2^32-1 (values above 2^31-1 are taken as their bit pattern), and likewise for `i64`. There is no `f32` literal: write `1.5 f32.demote_f64`, and in a test compare an `f32` result after `f64.promote_f32`, which is exact.

## 3. Numeric primitives

The numeric primitive set is **exactly the wasm numeric instruction set**, under the wasm names: `i32.add`, `i32.div_s`, `i32.div_u`, `i32.lt_u`, `i32.shr_s`, `f64.sqrt`, `i32.wrap_i64`, `f64.convert_i32_s`, `i32.load`, `i32.store8`, and so on. Effects follow directly from the instruction types.

Semantics are wasm's, including traps (integer divide by zero, overflowing float-to-int conversion). The language adds no checks and no abstractions here; the language reference points at the wasm specification for each instruction.

Comparisons return `i32` 0 or 1. Loads and stores take `i32` addresses with natural alignment and offset 0; stores take `( addr value -- )`. Explicit alignment/offset variants may come later. Loads and stores are raw words (section 8): only the prelude and words marked `raw` may use them.

## 4. Strings

### Representation

A `str` is an `( addr len )` pair: a byte address in linear memory and a byte length. Content is UTF-8. Strings are **immutable by convention**: literals live in read-only data, and no primitive writes through a `str`.

### Ownership

v1 uses a bump allocator with no free. Primitives that produce new bytes (`str.concat`) allocate and never release. Slicing allocates nothing. This is the allocator's limitation, accepted for v1, not a property of strings.

### Indexing is by byte

UTF-8 has no constant-time character indexing, and the language does not pretend otherwise. Offsets and lengths are bytes. A byte offset is a codepoint boundary if the byte there is not a continuation byte (`10xxxxxx`), which is one load and one mask to test.

### Primitives

| Word | Effect | Behaviour |
|---|---|---|
| `str.len` | `( str -- i32 )` | Byte length. |
| `str.byte-at` | `( str i32 -- i32 )` | Byte at offset. Traps if out of range. |
| `str.slice` | `( str i32 i32 -- str )` | `start len`, in bytes. **Checks and traps**: both ends in range, both on codepoint boundaries. No allocation. |
| `str.eq` | `( str str -- i32 )` | Byte-wise equality. |
| `str.concat` | `( str str -- str )` | Allocates a new string. |
| `str.cp-at` | `( str i32 -- i32 i32 )` | Decodes the codepoint at a byte offset; returns `codepoint bytelen`. Traps if the offset is not a boundary or the sequence is invalid. |

`print ( str -- )` is library code that writes to `/dev/cons` (section 10).

Everything else (codepoint iteration, codepoint counting, splitting, searching, number formatting and parsing) is library code written in the language. Grapheme clusters, normalisation and case mapping are out of scope.

### Literals in the REPL

Each REPL word is its own module importing shared memory, so a module cannot place its own data. At define time the host bump-allocates the literal's bytes and copies them in; the word references the resulting address. Export mode uses ordinary active data segments.

## 4a. Arrays

An `array T` is an `( addr len )` pair: a byte address and an element count. Elements are stored contiguously at their natural size and alignment (`str` elements are 8 bytes, two `i32`s). Arrays are mutable; `str` is not an array.

| Word | Effect | Behaviour |
|---|---|---|
| `array.new` | `( i32 -- array T )` | Allocates `n` zeroed elements. `T` must be fixed by a stack assertion or a local, since the result type is not otherwise known. |
| `array.len` | `( array T -- i32 )` | |
| `array.at` | `( array T i32 -- T )` | Bounds-checked; traps. |
| `array.at!` | `( array T i32 T -- )` | Bounds-checked; traps. |
| `array.slice` | `( array T i32 i32 -- array T )` | `start len`; bounds-checked; no allocation. |

**Collection combinators** take a quotation as syntax, like `times`, and inline it into a loop at the use site. They are primitives with type-variable effects, resolved at each use against the concrete stack.

| Word | Effect | Notes |
|---|---|---|
| `each` | `( array T [ T -- ] -- )` | |
| `map` | `( array T [ T -- U ] -- array U )` | Allocates the result. |
| `filter` | `( array T [ T -- i32 ] -- array T )` | Allocates the result. |
| `fold` | `( array T U [ U T -- U ] -- U )` | |

The quotation body's effect is checked exactly as for `if` and `while`. `leave` is permitted inside `each` and `fold`.

```
: doubled ( array i32 -- array i32 )  [ 2 i32.mul ] map ;
: sum ( array i32 -- i32 )  0 [ i32.add ] fold ;
```

## 4b. Structs

`struct point  x: i32  y: f64` declares a struct: the name, then `field: type` pairs while the next token is a field label, with no terminator. A field type is any type, including a struct declared above, the struct itself, or `array` of it.

The declaration generates ordinary words: `point.new ( i32 f64 -- point )` (fields in order), and for each field `point.x ( point -- i32 )` and `point.x! ( point i32 -- )`. They are listed by `words` and appear in the dependency graph.

Lowering: a WasmGC struct type, every field mutable. A `str` or linear `array` field is two `i32` fields, an `array <struct>` field three (reference, start, count), a struct field one reference, a quotation one `i32`. Each struct's type and the GC array type of its elements form one rec group.

Rules: a struct can be named only after its declaration; redeclaring with the same fields is a no-op and with different fields is `E_REDEFINE_EFFECT`; field names `new` and names ending in `!` are reserved; struct values are not test literals. An `array <struct>` is a view `( ref start count )` over a WasmGC array: the array words and combinators work on it, and slicing shares storage. `array.new` fills it with null references, and reading a field of one traps; programs cannot otherwise make or test a null.

## 4c. Unions and generic types

**Declarations.**

```
struct pair T U  first: T  second: U
union shape
  | circle  r: f64
  | rect    w: f64  h: f64
  | empty
union option T | none | some  v: T
```

`struct name P...  field: type ...` and `union name P...  | variant  field: type ...  | variant ...`. Optional type parameters are uppercase-initial names listed between the name and the first field or first `|`; there is no terminator, as for `struct`. A union needs at least one variant, and a variant may have no fields. A struct or union name may be the name of a primitive word (`map`), since types and words are separate namespaces, but not a type keyword (`i32`, `str`, `array`, ...). Variant names follow struct-name rules (lowercase-initial, not a primitive or type keyword) and may not be `else` or `tag`. Field names `new` and names ending in `!` are reserved, as for structs. A parameter, variant, or a field within one variant named twice is `E_SYNTAX`.

A field type may name only the declared parameters; any other type variable is `E_UNKNOWN_TYPE`, naming the parameter list. A generic type may name itself in its fields only applied to exactly its own parameters in order (`tail: list T` inside `union list T`); any other self-application is `E_UNKNOWN_TYPE`. A non-generic struct or union may name itself directly, in `array`, or through a generic type declared above (`next: option node`). Struct and union names share one namespace, separate from words, and a type can be named only after its declaration.

**Type application.** A generic type is written prefix with exactly as many argument types as it has parameters, like `array T`: `pair i32 str`, `option node`, `array pair i32 str` (an array of pairs), `pair list i32 str` (a pair whose first component is `list i32`). There are no parentheses: the parser reads the arguments by the declared arity, so the declaration must come first, and the wrong number of arguments is `E_UNKNOWN_TYPE`. Types display the same way. Applied types may appear wherever a type may: effects, stack assertions, fields, `array` elements, quotation types, and `( option T )` inside a generic word.

**Generated words.** A struct generates `s.new`, `s.f` and `s.f!` as in section 4b. A union generates:

| Word | Effect | Behaviour |
|---|---|---|
| `shape.circle` | `( f64 -- shape )` | A constructor per variant, its fields in order. |
| `shape.empty` | `( -- shape )` | |
| `shape.tag` | `( shape -- i32 )` | The variant's index in declaration order, from 0. |
| `shape.circle.r` | `( shape -- f64 )` | A reader per field of each variant. Traps with `shape.circle.r: not a circle` when the value is another variant. |

There are no per-variant writers: fields are reached through `match`. `tag` and the readers exist for code that has already tested the variant and for the REPL's stack echo. For a generic type every generated word is a generic word (section 18): `pair.new ( T U -- pair T U )`, `option.some ( T -- option T )`, `option.tag ( option T -- i32 )`, `option.some.v ( option T -- T )`, instantiated per concrete use and named `pair.new<i32,str>` by `words`, `deps` and `dead`. A constructor's type arguments are inferred from the stack (`3 "x" pair.new` is a `pair i32 str`); a use that nothing fixes (`option.none` alone) is `E_AMBIGUOUS_TYPE`, as for any generic word. Generated words are listed by `words` as `generated`, are never reported dead, and cannot be forgotten or forced.

**Monomorphisation.** Each concrete instantiation is its own WasmGC type: `pair i32 str` and `pair str i32` are distinct types that share nothing. An instantiation is made when it is first needed. A union value is never null; nulls arise only as unset elements of `array.new` over a struct, union or instantiated type, and `tag`, a reader or `match` on one traps with a null-reference message.

**match.**

```
s circle: [ :> r  r r f64.mul 3.14 f64.mul ]
  rect:   [ f64.mul ]
  else:   [ drop 0.0 ]
  match
```

Labelled arms are written directly before the word `match`, in any order, as `if` takes its quotations. Arms are syntax: they are inlined, the word's locals are visible in them, and `leave` in an arm inside a loop works as in `if`. The union is the type of the value on top of the stack; `match` on a value that is not a union is `E_TYPE_MISMATCH`, and on a value whose type is not yet known (as in an un-annotated word that matches its input) it is `E_AMBIGUOUS_TYPE`: write the effect or an assertion. An arm named for a variant receives, in place of the value, that variant's fields in declaration order (the last on top). `else:` is an optional catch-all for every variant not named, and receives the whole union value. Every arm that does not diverge must leave the same stack (`E_BRANCH_MISMATCH`, naming the two arms), as for `if`; an arm that diverges (`trap`, `leave`) is exempt. Arms are tested in the order written.

`E_MATCH_ARM`: an arm names something that is not a variant of the union, names a variant twice, `else:` appears twice, or `else:` is present when every variant is already named. `E_MATCH_MISSING`: a variant has no arm and there is no `else:`; the diagnostic's `expected` lists the missing variants. Both messages name the union and its variants.

**Redeclaration.** Declaring a struct or union again with the same parameters (same names, same order) and the same fields or variants is a no-op. Anything else, including a reordered parameter list or a union where a struct was, is `E_REDEFINE_EFFECT`, listing the dependants. `)force` chunks may not hold `struct` or `union`.

**REPL echo.** A union value prints as `shape.circle{r: 1.5}`, a fieldless variant as `shape.empty{}`, and a value of an instantiated type under its applied type name: `( option i32 ) option.some{v: 3}`. Nesting shows three levels, then `name{...}`. A stack holding a struct or union value prints one entry per line, as for structs.

**Prelude.** `union option T | none | some  v: T` is in the prelude; an optional value is an `option T`. Union and generic struct values are not test literals: tests compare through words.

## 4d. Equality, hashing and collections

**`eq` and `hash`.**

| Word | Effect | Behaviour |
|---|---|---|
| `eq` | `( a a -- i32 )` | 1 when the two values are equal by contents, else 0. Both operands have one type. |
| `hash` | `( a -- i32 )` | A hash of the contents: values `eq` reports equal hash alike. |

Both are polymorphic primitives like `dup`, instantiated at each use. They may be used on a `T` inside a generic word and are resolved per instance; on a value whose type is not yet known they are `E_AMBIGUOUS_TYPE`. They work by contents:

| Type | Equal when |
|---|---|
| `i32`, `i64` | same value |
| `f32`, `f64` | same bit pattern: NaN equals itself, `0.0` differs from `-0.0` |
| `str` | same bytes |
| `array T` | same length and elements equal in order |
| a struct | every field equal, recursively |
| a union | same variant and its fields equal |
| a function value | same table slot: two quotations with the same text are different values |

`hash` follows the same structure. Its only promise is that equal values hash alike; the function itself is not a contract and may change between versions. A cyclic struct value makes `hash` and `eq` loop forever: there is no cycle detection. Numbers and function values compile to inline code; every other type gets a generated word per concrete type (`eq<point>`, `hash<array str>`), made on first use, listed by `words` as a library word and never reported dead.

**`vec T`.** A growable array in the prelude:

```
struct chunk T  items: array T
struct vec T  chunks: array chunk T  count: i32
```

Its storage is a list of chunks whose sizes double (1, 2, 4, ...), so storage never moves: element `i` is in chunk `31 - clz(i + 1)` at offset `i + 1 - 2^chunk`, found in constant time.

| Word | Effect | Behaviour |
|---|---|---|
| `vec.make` | `( -- vec T )` | An empty vec. Needs a context that fixes `T`: `vec.make ( vec i32 )`. |
| `vec.push` | `( vec T T -- )` | Append. |
| `vec.len` | `( vec T -- i32 )` | The number of elements. |
| `vec.at` | `( vec T i32 -- T )` | Read; traps `vec.at: index out of range`. |
| `vec.at!` | `( vec T i32 T -- )` | Write; traps `vec.at!: index out of range`. |
| `vec.pop` | `( vec T -- option T )` | Remove and return the last element, `option.none` when empty. |
| `vec.clear` | `( vec T -- )` | Length to 0, chunks kept, for reuse. |
| `vec.to-array` | `( vec T -- array T )` | A copy. |
| `vec.each` | `( vec T [ T -- ] -- )` | Call the function value on each element in order. |
| `vec.fold` | `( vec T A [ A T -- A ] -- A )` | Fold from the initial value. |

The function arguments are function values (`'word`), since quotation values take no inputs and capture no locals; a loop body that needs the word's locals goes through `vec.to-array [ ... ] each`. The generated words of the two structs (`vec.new`, `vec.chunks`, `vec.count`, `chunk.*`) are an implementation detail; `vec.new` is the generated constructor, which is why the public one is `vec.make`.

**`map K V`.** A hash map in the prelude, open addressing with linear probing over `hash` and `eq`:

```
struct map K V  ks: array K  vs: array V  used: array i32  count: i32  filled: i32
```

The capacity is a power of two from 8; `used` holds 0 for an empty slot, 1 for a live one and 2 for a removed one. Before an insert would take the filled slots past half the capacity, the map is rehashed into arrays twice as large.

| Word | Effect | Behaviour |
|---|---|---|
| `map.make` | `( -- map K V )` | An empty map. Needs a context that fixes `K` and `V`: `map.make ( map str i32 )`. |
| `map.set` | `( map K V K V -- )` | Insert or replace. |
| `map.get` | `( map K V K -- option V )` | The value, or `option.none`. |
| `map.has` | `( map K V K -- i32 )` | 1 when the key is present. |
| `map.remove` | `( map K V K -- )` | Remove the key if present. |
| `map.len` | `( map K V -- i32 )` | The number of keys. |
| `map.keys` | `( map K V -- vec K )` | The keys, in no specified order. |
| `map.each` | `( map K V [ K V -- ] -- )` | Call the function value on each entry, in no specified order. |
| `map.find` | `( map K V K -- i32 )` | The probe the others use: the key's slot, or -1. |

`map.put` (store a key known to be absent) and `map.rehash` (move every entry into arrays twice as large) are the implementation of `map.set`, as are the generated words of the struct (`map.new`, `map.ks`, ...).

A key changed after insertion is lost: its hash no longer matches its slot.

**Memory.** Collections live in linear memory, which is never freed (`FUTURE.md` section 4). A vec's growth abandons nothing, since chunks never move; a map's rehash abandons the old arrays; a dropped collection is never reclaimed. `vec.clear` is the way to reuse one.

**REPL echo.** A vec or map value echoes as the struct it is: `vec{chunks: <32 elements>, count: 3}`.

## 4e. Byte buffers

A `bytes` is an `( addr len )` pair like a `str`, but mutable and not text: the buffer `host.read` fills. Its words check the length: `bytes.at`, `bytes.at!` and `bytes.slice` trap out of range, and `host.read` fills at most `bytes.len` bytes, so a read cannot overrun its buffer. `bytes.new ( i32 -- bytes )` allocates zeroed bytes; `bytes.to-str` copies; `bytes.as-str` gives the buffer itself as a string, without copying, for a buffer that is finished with. `bytes.put ( bytes i32 str -- )` copies a string in at an offset, and `bytes.u32-at`, `bytes.u64-at` and their `!` forms read and write little-endian numbers at a byte offset, for binary formats; all trap out of range. `bytes.len`, `bytes.addr` and `bytes.from-raw` are primitives; the rest is prelude. `eq` and `hash` compare by contents, as for `str`.

## 5. Shuffle primitives

Built-in polymorphic words. Their effects use type variables, resolved at each use against the stack (see `ARCHITECTURE.md` 5b). User words may be polymorphic too: their effects may name type variables, and each concrete use is compiled as its own instance (section 18).

| Word | Effect |
|---|---|
| `dup` | `( a -- a a )` |
| `drop` | `( a -- )` |
| `swap` | `( a b -- b a )` |
| `over` | `( a b -- a b a )` |
| `nip` | `( a b -- b )` |
| `tuck` | `( a b -- b a b )` |
| `rot` | `( a b c -- b c a )` |
| `-rot` | `( a b c -- c a b )` |
| `2dup` | `( a b -- a b a b )` |
| `2drop` | `( a b -- )` |
| `eq` | `( a a -- i32 )`, by contents (section 4d) |
| `hash` | `( a -- i32 )`, by contents (section 4d) |

Shuffles act on checker types: `dup` on a `str` copies both lowered halves.

## 6. Locals

Factor-style, one binding at a time.

| Form | Meaning |
|---|---|
| `:> name` | Pop the top of the stack into an immutable local, typed by what was popped. |
| `:> name!` | As above, but mutable. |
| `name` | Push the local's value. |
| `name!` | Pop the top of the stack into a mutable local. Type must match. Error on an immutable local. |

A local is scoped to the word it is bound in, visible inside quotations in the same word, and never captured. Binding the same name twice in one word is an error. Locals are not visible to tests.

```
: sum-to ( i32 -- i32 )
  :> n
  0 :> acc!
  n [ acc i32.add acc! ] times
  acc ;
```

## 7. Control flow

Factor-style quotations and combinators, as specified in `ARCHITECTURE.md` 5c: `if`, `when`, `unless`, `while`, `until`, `times`, `leave`, plus the collection combinators in section 4a. A quotation that is the argument of a combinator is syntax and is inlined. A quotation anywhere else is a value (section 7a).

`times` pushes the iteration index (an `i32`, counting from 0) at the start of each iteration; the body must consume it, so its effect is `( i32 -- )` relative to the surrounding stack.

## 7a. Functions as values

A word's address is its index in the shared function table, an `i32`, typed in the checker as a **quotation type** `[ inputs -- outputs ]`.

| Form | Effect | Notes |
|---|---|---|
| `'name` | `( -- [ effect of name ] )` | Takes a word's address. Records an address-taken edge in the dependency graph. |
| `[ body ]` not under a combinator | `( -- [ effect ] )` | An anonymous word; its effect is derived by forward checking from an empty stack, so the body must be self-contained. |
| `call` | `( ..inputs [ inputs -- outputs ] -- ..outputs )` | `call_indirect` with the matching wasm type. |

Quotation types may appear in word effects, so user words can take and return functions. A quotation type may contain type variables inside a generic effect: `( T [ T -- T ] -- T )`. `'word` on a generic word needs a context that fixes its instantiation, such as a stack assertion or a declared effect.

Quotation values do **not** capture locals. A quotation value that names a local of the enclosing word is an error. Closures are not v1 (`FUTURE.md`).

```
: twice ( i32 [ i32 -- i32 ] -- i32 )  :> f  f call f call ;
: inc ( i32 -- i32 )  1 i32.add ;
test twice : 5 'inc twice -> 7
```

## 8. Definitions, declarations, tests

Semantics as in `ARCHITECTURE.md` section 6.

```
: square ( i32 -- i32 ) dup i32.mul ;

declare parse-header ( str -- i32 i32 )

test square : 3 square -> 9
test parse-header : "abc" parse-header -> 3 0
```

- `raw : name ( effect ) body ;` (or `export raw :`) defines a word whose body may use the **raw words**, the ones that reach memory by address: loads and stores, `mem.alloc`, `memory.copy`, `memory.fill`, `str.addr`, `str.from-raw`, `bytes.addr`, `bytes.from-raw`. Elsewhere, outside the prelude, they are `E_RAW`, tests and REPL lines included. Quotations inside a `raw` word may use them too. Being `raw` is not part of the effect: callers need no marking.
- `: name ( effect ) body ;` defines a word; the effect immediately follows the name. The effect may be omitted, `: name body ;`, and is then inferred (section 18). Recursive and mutually recursive words, `export` words and `main` must write it.
- `export : name ( effect ) body ;` additionally exports the word from the built module and makes it a reachability root. `main ( -- )` is the entry point for `run`.
- `declare name ( effect )` creates a stub (see `ARCHITECTURE.md` 6).
- `test word : body -> expected` runs `body` on an empty stack and compares the resulting stack against `expected`, which must be literals, type by type. A test on a declared-but-undefined word is reported as pending. Naming the word lets tooling find a word's tests and lets `forget` remove them.

## 9. Effects, stack assertions, comments

- **Effect**: `( inputs -- outputs )`, types separated by whitespace, top of stack rightmost. Distinguished by the `--` token.
- **Stack assertion**: parentheses **without** `--`, listing the full stack as types, top rightmost. The checker verifies it at that point in the body. A Forth-style `( a b )` with names is an error ("unknown type `a`"). A name starting with an uppercase letter is a type variable (`T`, `U`, `Elem`), allowed in effects and, within a generic word, in assertions.
- **Comment**: `#` to end of line. Parentheses are never comments.

```
: hypot ( f64 f64 -- f64 )
  dup f64.mul          ( f64 f64 )   # x y*y
  swap dup f64.mul     ( f64 f64 )
  f64.add f64.sqrt ;
```

## 10. I/O

Four host imports, specified in `ARCHITECTURE.md` 5d: `host.open ( str i32 -- i32 )`, `host.read ( i32 bytes -- i32 )`, `host.write ( i32 str -- i32 )`, `host.close ( i32 -- i32 )`. All other I/O is a path in the namespace.

**Modes** for `host.open`: `0` read, `1` write (truncate), `2` append, `3` read-write. **Error codes** are negative `i32`: `-1` not found, `-2` permission, `-3` not supported on this host, `-4` I/O error, `-5` bad handle, `-6` malformed request. Further codes may be added; programs should treat any negative value as failure.

**Directory records**, as returned by reading a directory handle, each record in order: `u32` name byte length, name bytes (UTF-8), `u64` size, `u8` is-dir flag. Records are packed with no padding. A read may return any whole number of records; a partial record is never returned.

Library words built on this, shipped with the language: `print`, `read-line`, `read-file`, `write-file`, `copy`, `ls`, `now`.

## 11. Open items

1. Alignment and offset immediates on loads and stores: v1 is natural alignment, offset 0 (section 3).
2. Module or namespace structure, if any, for libraries: a flat dictionary until it hurts.

## 12. Decisions taken in M1

Recorded here so the spec matches the compiler. `docs/reference.md` is the user-facing summary.

1. **Low-level primitives** added so that library code can be written in Chasm: `str.addr ( str -- i32 )`, `str.from-raw ( i32 i32 -- str )` (unchecked), `mem.alloc ( i32 -- i32 )` (bump, zeroed, 8-byte aligned), and `trap ( str -- )`, which stops the program with a message.
2. **Prelude.** `str.byte-at`, `str.slice`, `str.eq`, `str.concat`, `str.cp-at` are library words written in Chasm on top of those primitives, compiled with every program. So are `str.boundary? ( str i32 -- i32 )`, `i32.to-str`, `i64.to-str`, `f64.fixed`, `str.from-byte`, `array.to-str`, `print`, `println`, `read-line`, `read-file`, `write-file`, `copy`, `ls` and `now`. `chasm words` lists them.
3. **Library word effects.** `read-line ( -- str i32 )` (flag 0 at end of input), `read-file ( str -- str i32 )` (namespace path; status 0 or an error code), `write-file ( str str -- i32 )` (contents, path; replaces the file; status), `copy ( str str -- i32 )` (from, to; status), `ls ( str -- i32 )` (prints entries; status), `now ( -- i64 )` (nanoseconds since the Unix epoch).
4. **`/dev/time`** reads 8 bytes: a little-endian `u64` of nanoseconds since the Unix epoch, then end of file.
5. **Divergence.** `leave` and `trap` end the quotation they appear in; code after them is `E_UNREACHABLE`. A branch that diverges need not match the other branch, and a `when` body that diverges is accepted.
6. **`until`** runs its body, then its condition, and repeats until the condition is non-zero.
7. **Tests.** The expected part of `test word : body -> expected` is the maximal run of literal tokens after `->`. Tests are processed in file order, so a test must come after its word is defined or declared. Tests may name primitives. Each test runs in a fresh module instance with a captured console.
8. **Type variables** arise only from `array.new` and the element types of the array words. They are resolved by plain unification against later uses (assertions, locals, calls, the word's effect); if one is never fixed the error is `E_AMBIGUOUS_TYPE`. User effects may contain them too, written as uppercase-initial names (section 18).
9. **Quotation values** take no inputs, as specified in 7a: their effect is `( -- outputs )`. Functions with inputs are passed as `'word`.
10. **Redefinition in files.** Top-level forms are processed in order. A redefinition with the same effect replaces the body for every caller; the last one wins. A word may call itself.
11. **Names.** Primitive names cannot be defined, declared, or used as locals. Locals shadow user words.

## 13. Decisions taken in M2

1. **REPL input.** A chunk whose first token is `:`, `export`, `declare` or `test` is top-level forms, processed exactly as in a file. Any other chunk is one line: a body checked forward from the current types of the memory data stack. A line cannot follow a definition in the same chunk (`E_SYNTAX`).
2. **Continuation.** A chunk continues onto the next input line while a `:` definition or a `[` quotation is still open.
3. **Stack echo.** After each chunk the REPL prints the stack as `( types ) values`, bottom to top, for example `( i32 str ) 9 "hi"`, or `( )` when empty. Arrays print as `<n elements>` and function values as `#slot`.
4. **Tests at the REPL** run as soon as their word has a body, and again whenever the word is redefined; a test of a declared word waits for its body.
5. **Literals in the REPL** (section 4) are placed by the host at the heap pointer, one window per step; identical literals are shared within a step only.

## 14. Decisions taken in M3

1. **Literal types and conversion words.** No literal has a type suffix. An integer is an `i32`; an integer token followed directly by the word `i64` is a single `i64` literal (`42 i64`, also in a test's expected values), written as the value it means, so `4294967295 i64` is 4294967295. Applied to any other `i32`, `i64` is a prelude word `( i32 -- i64 )` that sign-extends. A type name is a conversion word only for an exact widening from one source type; every lossy conversion (`i32.wrap_i64`, `i32.trunc_f64_s`, `f32.demote_f64`, ...) keeps its wasm name, so the reader sees how the value is cut. Values print the same way: `120 i64`.
2. **What goes in the prelude.** A word joins the prelude when the example corpus has written it by hand more than once and any program might want it: `f64.fixed ( f64 i32 -- str )`, `array.to-str ( array i32 -- str )` and `str.from-byte ( i32 -- str )` came in this way. Prelude names are `type.verb`, because the dictionary is flat: a program may redefine a prelude word only with the same effect. Code that only some programs want (test helpers, for instance) is a question for library structure (`ARCHITECTURE.md` open question 2), not the prelude.
3. **No include form.** A program is the files named on the command line, in order, compiled as one dictionary; the shell composes programs (`chasm test lib.chasm prog.chasm`, or `cat` into `/dev/stdin`). The core keeps no I/O and the language no mechanism for it. Files in `examples/` stay self-contained, so the examples gate can check each one alone.

4. **REPL commands are outside the language.** `)forget word` and any later REPL command start with `)`, which no Chasm line can; `forget` is not a keyword, so no file can contain it.
5. **Dead words** are those that `main` and the `export` words never reach; tests do not keep a word alive.

## 15. Decisions taken in M4

1. **Source form and generated words.** `struct point  x: i32  y: f64`, no terminator; it generates `point.new`, `point.x` and `point.x!` per field as ordinary dictionary words.
2. **One rec group per struct.** Each struct's type and its GC array type form one rec group, so a struct may hold itself and an `array` of itself; it may refer only to structs declared earlier, so mutual recursion is rejected (`E_UNKNOWN_TYPE`).
3. **Nullable references.** Locals, fields and array elements hold `(ref null $T)`. Nulls arise only as unset elements of a fresh `array.new`, and reading a field of one traps.
4. **Arrays of structs are views** `( ref start count )` over a WasmGC array, so slicing is free and shares storage, as for all arrays.
5. **Struct values are not test literals**; tests compare fields.
6. **REPL echo.** `point{x: 7, y: 2.5}`, and a stack holding a struct prints one entry per line (`point point{x: 7, y: 2.5}`) between `(` and `)`; nested structs to three levels, then `name{...}`; an unset struct is `null`; arrays `<n elements>`.
7. **Names.** Struct names and word names are separate namespaces; generated words follow the `name.field` rule and clash with user words only through the ordinary redefinition rule.
8. **Linear arrays are unchanged.** `array i32`, `array str` and the like stay `( addr count )` in linear memory.
9. **No null test.** Programs never see a null value. An optional link is an `array` of length 0 or 1 (`struct node  v: i32  next: array node`), tested with `array.len`.

## 16. Decisions taken in M5

1. **A built program has no reachable stubs.** `build` and `run` refuse a declared word without a body that `main` or an `export` word reaches (`E_UNRESOLVED`); `check` and `test` still accept it, so contract-first work goes on as before. Words no root reaches are left out of the module.

## 17. Decisions taken in M6

1. **`)force` is a REPL command outside the language**, like `)forget` (section 14.4); no file can contain it. It takes one or more definitions and optional `test` lines, ending at an empty line, and changes effects deliberately.
2. **Refuse and list.** If any dependant of a changed word fails to check against the new effect, nothing changes and `E_FORCE` (a new stable code) lists the broken dependants. A forced word's own tests are dropped, so its new contract's tests go in the same chunk.
3. **`/net/http/<host>[:port]/<path>`** (and `/net/https/...` for TLS) is an HTTP request. What is written to the handle is the rest of the request after its first line: `Name: value` header lines, an empty line, the body, so `"Accept: text/plain\n\n"` is one header and no body. The request is sent at the first read: nothing written is a GET, a header block with an empty body is a GET with those headers, and a non-empty body makes a POST. The read returns the response body only. Status 404 is `-1`, 401 and 403 are `-2`, any other status outside 200 to 299 or a failed connection is `-4`, and a malformed header block is `-6` (a new code). The browser host drops header names the Fetch standard forbids (`Host`, `Content-Length`, ...).

## 18. Decisions taken in M7

1. **Optional effects.** A definition may omit its effect. It is inferred as the smallest number of inputs for which the body checks, with any type left open generalised to a type variable, and the result is checked exactly as a written effect. Recursive and mutually recursive words, `export` words and `main` must write their effects (`E_NEEDS_EFFECT`, a new stable code).
2. **Type variables** are identifiers starting with an uppercase ASCII letter (`T`, `U`, `Elem`), usable in effects, in `array T`, in quotation types `[ T -- T ]` and, inside a generic word, in stack assertions. Built-in types and struct names are lowercase; an uppercase struct name is `E_SYNTAX`, and struct fields cannot be type variables.
3. **Generic words** are checked once with their type variables rigid: a `T` matches only `T`, so there are no constraints, and `i32.add` on a `T` is `E_TYPE_MISMATCH`. Each concrete use compiles an instance, named `twice<i32>` in `chasm words`, `chasm dead` and `chasm deps`. A test of a generic word runs on the instance its body uses. `export` words and `main` are concrete.
4. **Redefinition** keeps the same-effect rule for inferred and generic effects alike, and a redefined generic word's instances are rebuilt.
5. **`chasm infer`** lists the effects it inferred; `--write` writes them into the source.

## 19. Decisions taken in M8

1. **Unions with inline variants**, `union name P... | variant  field: type ...`, generating a constructor per variant, `tag`, and a trapping reader per field (section 4c). No per-variant writers.
2. **`match` is postfix with labelled arms** in any order, written before the word as `if` takes its quotations; no closing keyword. Each arm receives its variant's fields; `else:` covers every variant not named and receives the whole value. Arms are tested in the order written.
3. **Type parameters are explicit and ordered**, listed after the name (`struct pair T U`), and generic types are applied prefix by arity (`pair i32 str`) with no parentheses. Reordering fields cannot change what an applied type means; reordering parameters is a redefinition.
4. **Generic structs and unions are monomorphised**: each concrete instantiation is its own WasmGC type, and their generated words are generic words instantiated per use.
5. **New codes**: `E_MATCH_ARM` and `E_MATCH_MISSING`.
6. **Byte identity.** Generic types and unused prelude unions put nothing in a module's type section until instantiated, so programs that use neither compile byte-identically to M7.
7. **`option T` is in the prelude** and replaces the array-of-0-or-1 idiom for optional links; decision 15.9 (no null test; optional links as arrays of 0 or 1) is superseded by it.

## 20. Decisions taken in M9

1. **`eq` and `hash` are by-contents polymorphic primitives**, instantiated per use like `dup`: inline for numbers and function values, a generated word per concrete type otherwise. Floats compare by bit pattern. There is no cycle detection, and the hash function is not a contract.
2. **`vec T` is a list of doubling chunks** in the prelude, so storage never moves and growth abandons nothing.
3. **`map K V` is open addressing** with linear probing in the prelude, rehashed by doubling before the filled slots pass half the capacity.
4. **`vec.make` and `map.make`** are the public constructors, since `.new` is the generated struct constructor.
5. **Type names may be primitive names** (`map`): types and words are separate namespaces. Type keywords stay reserved.
6. **Byte identity.** The prelude's generic structs and generic `:` words are made lazily, like `option` (decision 19.6), so programs that use none of vec, map, `eq` and `hash` compile byte-identically to M8.
7. **The memory leak is accepted**: collections live in never-freed linear memory, and `vec.clear` is the reuse path. Moving storage to the collector is `FUTURE.md` section 4.
8. **No compact echo**: vec and map values echo as structs.
9. **No new diagnostic codes.**
10. **Function arguments are function values** (`'word`): quotation values take no inputs, so `vec.each`, `vec.fold` and `map.each` take words.

## 21. Decisions taken after M9

1. **`bytes`, and host words that cannot overrun.** `host.read` was `( i32 i32 i32 -- i32 )`, a bare address and a length that nothing tied to the allocation, so a length larger than the buffer silently wrote past it. It is now `( i32 bytes -- i32 )`, and `host.write` is `( i32 str -- i32 )`: the length travels with the address. Both lower to the same `i32 i32 i32` as before, so the ring, the hosts and the generated code are unchanged; only the checker's effects differ. `bytes` follows `str`: three primitives (`bytes.len`, `bytes.addr`, `bytes.from-raw`), the checked words in the prelude, trap messages named after the word. `bytes.to-str` copies, because a `str` is immutable by convention and the buffer is not; `bytes.as-str` shares instead, since memory is never freed and a copy per string built in a buffer adds up. It stays memory-safe (the string has the buffer's bounds); a later write to the buffer shows through the string, which is the caller's promise not to do. No new diagnostic codes.
2. **The raw memory words are gated.** `mem.alloc`, loads and stores, `memory.copy`, `memory.fill`, `str.addr`, `str.from-raw`, `bytes.addr` and `bytes.from-raw` check nothing, so they are allowed only in the prelude (which is the trusted code to audit), in the generated `eq` and `hash` words, and in a word marked `raw :`; anywhere else they are `E_RAW`, a new code. `Ctx::raw` says whether the body being compiled may use them: `process_item` sets it per item from the origin and the marker, an instance takes it from its template, and a REPL line clears it. A word-level marker, not a file or command-line switch, so the unchecked code stays small and visible (`chasm words` flags it `[raw]`). `memory.size` and `memory.grow` stay open: they take and give no address. `bytes.put`, `bytes.u32-at`, `bytes.u64-at` and their stores were added so that no example needs `raw`; a binary format is read through them, checked.
3. **`bytes.as-str` is not raw.** It shares the buffer with the string, but the string has the buffer's bounds, so it is memory-safe; only the immutability of `str` is at the caller's word.
