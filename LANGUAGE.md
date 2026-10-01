# Chasm: Language Specification

Status: draft v0.7 (M1 decisions recorded; see section 12). Chasm source files use the `.chasm` extension. Companion to `ARCHITECTURE.md`. Sections marked **TBD** are not yet decided.

## 1. Types

| Type | Wasm lowering | Notes |
|---|---|---|
| `i32` | `i32` | Also used for booleans (0 false, non-zero true) and memory addresses (wasm32). |
| `i64` | `i64` | |
| `f32` | `f32` | |
| `f64` | `f64` | |
| `str` | `i32 i32` (addr, byte length) | Checker-level type with a fixed, documented lowering. See section 4. |
| `array T` | `i32 i32` (addr, element count) | `T` is any type in this table except `array`. See section 4a. |
| `[ effect ]` | `i32` (function table index) | A quotation type: a function as a value. See section 7a. |

Signedness is a property of operations, not types, as in wasm. There is no `bool`, `char`, `u32`, `v128` or nested array in v1.

A word's effect lists checker types. Its wasm function type is the effect with each type lowered, in order. So `( str i32 -- str )` has wasm type `(i32 i32 i32) -> (i32 i32)`.

## 2. Literals

| Form | Type | Example |
|---|---|---|
| decimal or hex integer, no suffix | `i32` | `42`, `-7`, `0xFF` |
| integer with `i64` suffix | `i64` | `42i64` |
| decimal with a point or exponent, no suffix | `f64` | `1.5`, `2e10` |
| float with `f32` suffix | `f32` | `1.5f32` |
| double-quoted | `str` | `"hello"` |

String literals are UTF-8, immutable, and live in read-only data. Escapes: `\"`, `\\`, `\n`, `\t`, `\u{XXXX}`. Integer literals that do not fit their type are a compile error; an unsuffixed integer may be anything from -2^31 to 2^32-1 (values above 2^31-1 are taken as their bit pattern), and likewise for `i64`.

## 3. Numeric primitives

The numeric primitive set is **exactly the wasm numeric instruction set**, under the wasm names: `i32.add`, `i32.div_s`, `i32.div_u`, `i32.lt_u`, `i32.shr_s`, `f64.sqrt`, `i32.wrap_i64`, `f64.convert_i32_s`, `i32.load`, `i32.store8`, and so on. Effects follow directly from the instruction types.

Semantics are wasm's, including traps (integer divide by zero, overflowing float-to-int conversion). The language adds no checks and no abstractions here; the language reference points at the wasm specification for each instruction.

Comparisons return `i32` 0 or 1. Loads and stores take `i32` addresses with natural alignment and offset 0; stores take `( addr value -- )`. Explicit alignment/offset variants may come later.

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

## 5. Shuffle primitives

The only polymorphic words in the language. Effects use type variables, resolved at each use against the concrete stack (see `ARCHITECTURE.md` 5b).

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

Quotation types may appear in word effects, so user words can take and return functions. User words remain monomorphic: `( array i32 [ i32 -- i32 ] -- array i32 )` is a concrete type.

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

- `: name ( effect ) body ;` defines a word. The effect is mandatory and immediately follows the name.
- `export : name ( effect ) body ;` additionally exports the word from the built module and makes it a reachability root. `main ( -- )` is the entry point for `run`.
- `declare name ( effect )` creates a stub (see `ARCHITECTURE.md` 6).
- `test word : body -> expected` runs `body` on an empty stack and compares the resulting stack against `expected`, which must be literals, type by type. A test on a declared-but-undefined word is reported as pending. Naming the word lets tooling find a word's tests and lets `forget` remove them.

## 9. Effects, stack assertions, comments

- **Effect**: `( inputs -- outputs )`, types separated by whitespace, top of stack rightmost. Distinguished by the `--` token.
- **Stack assertion**: parentheses **without** `--`, listing the full stack as types, top rightmost. The checker verifies it at that point in the body. A Forth-style `( a b )` with names is an error ("unknown type `a`").
- **Comment**: `#` to end of line. Parentheses are never comments.

```
: hypot ( f64 f64 -- f64 )
  dup f64.mul          ( f64 f64 )   # x y*y
  swap dup f64.mul     ( f64 f64 )
  f64.add f64.sqrt ;
```

## 10. I/O

Four host imports, specified in `ARCHITECTURE.md` 5d: `host.open`, `host.read`, `host.write`, `host.close`. All other I/O is a path in the namespace.

**Modes** for `host.open`: `0` read, `1` write (truncate), `2` append, `3` read-write. **Error codes** are negative `i32`: `-1` not found, `-2` permission, `-3` not supported on this host, `-4` I/O error, `-5` bad handle. Further codes may be added; programs should treat any negative value as failure.

**Directory records**, as returned by reading a directory handle, each record in order: `u32` name byte length, name bytes (UTF-8), `u64` size, `u8` is-dir flag. Records are packed with no padding. A read may return any whole number of records; a partial record is never returned.

Library words built on this, shipped with the language: `print`, `read-line`, `read-file`, `ls`, `now`.

## 11. Open items

1. Alignment and offset immediates on loads and stores: v1 is natural alignment, offset 0 (section 3).
2. Module or namespace structure, if any, for libraries: a flat dictionary until it hurts.

## 12. Decisions taken in M1

Recorded here so the spec matches the compiler. `docs/reference.md` is the user-facing summary.

1. **Low-level primitives** added so that library code can be written in Chasm: `str.addr ( str -- i32 )`, `str.from-raw ( i32 i32 -- str )` (unchecked), `mem.alloc ( i32 -- i32 )` (bump, zeroed, 8-byte aligned), and `trap ( str -- )`, which stops the program with a message.
2. **Prelude.** `str.byte-at`, `str.slice`, `str.eq`, `str.concat`, `str.cp-at` are library words written in Chasm on top of those primitives, compiled with every program. So are `str.boundary? ( str i32 -- i32 )`, `i32.to-str`, `i64.to-str`, `print`, `println`, `read-line`, `read-file`, `ls` and `now`. `chasm words` lists them.
3. **Library word effects.** `read-line ( -- str i32 )` (flag 0 at end of input), `read-file ( str -- str i32 )` (namespace path; status 0 or an error code), `ls ( str -- i32 )` (prints entries; status), `now ( -- i64 )` (nanoseconds since the Unix epoch).
4. **`/dev/time`** reads 8 bytes: a little-endian `u64` of nanoseconds since the Unix epoch, then end of file.
5. **Divergence.** `leave` and `trap` end the quotation they appear in; code after them is `E_UNREACHABLE`. A branch that diverges need not match the other branch, and a `when` body that diverges is accepted.
6. **`until`** runs its body, then its condition, and repeats until the condition is non-zero.
7. **Tests.** The expected part of `test word : body -> expected` is the maximal run of literal tokens after `->`. Tests are processed in file order, so a test must come after its word is defined or declared. Tests may name primitives. Each test runs in a fresh module instance with a captured console.
8. **Type variables** arise only from `array.new` and the element types of the array words. They are resolved by plain unification against later uses (assertions, locals, calls, the word's effect); if one is never fixed the error is `E_AMBIGUOUS_TYPE`. User effects never contain them.
9. **Quotation values** take no inputs, as specified in 7a: their effect is `( -- outputs )`. Functions with inputs are passed as `'word`.
10. **Redefinition in files.** Top-level forms are processed in order. A redefinition with the same effect replaces the body for every caller; the last one wins. A word may call itself.
11. **Names.** Primitive names cannot be defined, declared, or used as locals. Locals shadow user words.

