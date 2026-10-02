# Chasm language reference

This is the working reference for writing Chasm. It describes what the
compiler in this repository accepts today. The design documents
(`ARCHITECTURE.md`, `LANGUAGE.md`) say why; this says how.

Chasm is a typed, concatenative language in the Forth and Factor family. A
program is a sequence of **words**. Every word declares its **stack effect**,
and the checker verifies each body against it before anything runs. Words
compile to WebAssembly functions whose types are their effects.

## 1. Running things

```
chasm check  FILE...          # types and effects only; fast
chasm run    FILE...          # build and run `main ( -- )`
chasm test   FILE...          # run the `test` lines
chasm unresolved FILE...      # declared words with no body yet
chasm words  FILE...          # every word and its effect
chasm deps WORD FILE...       # what WORD calls (--all for transitive)
chasm used-by WORD FILE...    # what calls WORD
chasm dead   FILE...          # words `main` and `export` words never reach
chasm build  FILE... -o out.wasm
chasm repl                    # interactive; reads chunks from stdin
```

A program is the files you name, in order; there is no include form. To use
a library, name it first: `chasm test lib.chasm prog.chasm`. A pipeline
works too, `cat lib.chasm prog.chasm | chasm test /dev/stdin`, but then error
locations count lines in the concatenated text.

Add `--json` to any command for a machine-readable report:

```json
{ "schema": 1, "ok": false, "command": "check",
  "diagnostics": [ { "code": "E_EFFECT_MISMATCH", "severity": "error",
                     "message": "...", "word": "f",
                     "location": { "file": "f.chasm", "line": 3, "column": 1, "token": "f" },
                     "expected": ["i32"], "actual": ["i32", "i32"] } ],
  "results": { ... } }
```

Codes are stable; messages may change. See section 15 for the list.

`run`, `test` and `repl` take `--mount NAME=DIR` (exposes `DIR` as
`/mnt/NAME`) and `--no-file` (hides the host filesystem). `repl` also takes
`--json` (one report per chunk, one per line) and `--no-prelude`.

## 1a. The REPL

`chasm repl` reads chunks from stdin and compiles and runs each at once.

- A chunk that starts with `:`, `export`, `declare` or `test` is processed
  exactly as in a file. Anything else is a **line**: it runs on the current
  stack, whose types are always known, and its effect is worked out from
  that stack. A line cannot follow a definition in the same chunk.
- A chunk continues on the next line while a `:` definition or a `[`
  quotation is open.
- After each chunk the stack is printed as `( types ) values`, bottom to
  top, or `( )` when empty. Arrays print as `<n elements>`, function
  values as `#slot`, structs with their fields, `point{x: 7, y: 2.5}`
  (nested structs to three levels, then `seg{...}`), and an unset struct
  as `null`. A stack holding a struct prints one entry per line, type then
  value, between `(` and `)`:

  ```
  (
  point point{x: 7, y: 2.5}
  i32 7
  )
  ```
- A `test` runs at once (`PASS` or `FAIL`), or, for a declared word, as soon
  as the word gets a body. A word's tests run again when it is redefined.
- Redefining a word with the same effect takes effect for every existing
  caller; a different effect is `E_REDEFINE_EFFECT` and lists the
  dependants.
- A line that traps prints `trap in `[line N]`: message` and leaves the
  stack as it was.
- A line starting with `)` is a REPL command, not Chasm, so a file never
  holds one. `)forget word` removes a word and its tests and frees the name,
  which can then be defined with any effect. It is refused (`E_FORGET`, with
  the `dependants`) while another word, a quotation in one, or another
  word's test uses it: forget those first, top-down. Primitives, prelude
  words and struct-generated words cannot be forgotten. A function value of
  a forgotten word already on the stack still runs the old code.
- `print` writes to the terminal and `read-line` reads from the same stdin
  as the REPL. With `--json` the program's output is captured into
  `results.output`; each report also has `results.defined`, `results.forgotten`,
  `results.tests`, `results.trap`, `results.stack` and `results.timing`.

```
> : sq ( i32 -- i32 ) dup i32.mul ;
ok: sq ( i32 -- i32 )
( )
> test sq : 3 sq -> 9
PASS     sq
( )
> 3 sq
( i32 ) 9
> 1 0 i32.div_s
trap in `[line 4]`: wasm trap: integer divide by zero
( i32 ) 9
> : twice ( i32 -- i32 ) sq sq ;
ok: twice ( i32 -- i32 )
( i32 ) 9
> twice
( i32 ) 6561
```

The same REPL runs in the browser; see `web/README.md`.

## 2. Program structure

A file is a sequence of top-level forms, processed **in order**:

```
: name ( inputs -- outputs )  body ;          # define
export : name ( inputs -- outputs )  body ;   # define and export from the module
declare name ( inputs -- outputs )            # stub: a contract without a body
test name : body -> expected-literals         # a test of `name`
```

- A word can call itself, and any word defined or declared **above** it.
  To call a word defined further down, `declare` it first.
- `#` starts a comment to end of line. Parentheses are never comments.
- Tokens are separated by whitespace: `[ dup ]`, not `[dup]`.
- The program's entry point is `: main ( -- ) ... ;`.

## 3. Types and effects

| Type | Meaning | Wasm |
|---|---|---|
| `i32` `i64` `f32` `f64` | numbers; `i32` is also boolean (0 false) and address | itself |
| `str` | immutable UTF-8 bytes | `i32 i32` (addr, byte length) |
| `array T` | mutable, fixed length; `T` not an array | `i32 i32` (addr, count); for a struct `T`, `ref i32 i32` (a view over a WasmGC array: ref, start, count) |
| `name` | a declared struct (section 10a): a reference to a garbage-collected record | `(ref null $name)` |
| `[ ins -- outs ]` | a function value | `i32` (table index) |

An effect lists types bottom to top, rightmost on top:
`( str i32 -- i32 i32 )` takes a `str` with an `i32` above it and leaves two
`i32`s. Words are monomorphic; only the built-in shuffles and combinators are
polymorphic.

## 4. Literals

| Form | Type |
|---|---|
| `42` `-7` `0xFF` | `i32` (any value from -2^31 to 2^32-1; large values wrap to their bit pattern) |
| `42 i64` | `i64`: an integer then the word `i64` is one literal, up to 2^64-1 |
| `1.5` `2e10` | `f64` |

There is no `f32` literal: `1.5 f32.demote_f64`.
| `"text"` | `str`; escapes `\" \\ \n \t \u{1F600}` |

## 5. Stack shuffles

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

These work on any type; `dup` on a `str` copies both halves.

## 6. Locals

Beyond three or so values, bind names instead of shuffling:

```
: sum-to ( i32 -- i32 )
  :> n            # pop into immutable local n
  0 :> acc!       # pop into mutable local acc
  n [ acc i32.add acc! ] times
  acc ;
```

- `:> x` pops into `x`; `:> x!` makes it mutable.
- `x` pushes its value; `x!` pops into it (mutable only).
- Locals are visible inside `[ ]` under combinators (`if`, `times`, ...) but
  **not** inside quotation values (no closures).
- Each name is bound at most once per word.

## 7. Numeric and memory primitives

Exactly the WebAssembly instructions, under their wasm names, with wasm
semantics (including traps on integer divide by zero):

- `i32.add i32.sub i32.mul i32.div_s i32.div_u i32.rem_s i32.rem_u i32.and i32.or i32.xor i32.shl i32.shr_s i32.shr_u i32.rotl i32.rotr i32.clz i32.ctz i32.popcnt i32.extend8_s i32.extend16_s`
- `i32.eqz i32.eq i32.ne i32.lt_s i32.lt_u i32.gt_s i32.gt_u i32.le_s i32.le_u i32.ge_s i32.ge_u` (return `i32` 0 or 1)
- The same set for `i64`, plus `i64.extend32_s`.
- `f32`/`f64`: `add sub mul div min max copysign abs neg ceil floor trunc nearest sqrt eq ne lt gt le ge`
- Widening: `i64 ( i32 -- i64 )` sign-extends, exactly. It is the only conversion with a short name; lossy ones keep their wasm names so the reader sees how the value is cut.
- Conversions: `i32.wrap_i64`, `i64.extend_i32_s`/`_u`, `iNN.trunc_fMM_s`/`_u`, `iNN.trunc_sat_fMM_s`/`_u`, `fNN.convert_iMM_s`/`_u`, `f32.demote_f64`, `f64.promote_f32`, `*.reinterpret_*`.
- Memory: `i32.load` ... `i64.load32_u` take `( i32 -- T )`; stores take `( i32 T -- )` (address below value). Natural alignment, offset 0. `memory.size ( -- i32 )`, `memory.grow ( i32 -- i32 )`, `memory.copy ( dst src n -- )`, `memory.fill ( dst byte n -- )`.

Note there is no `i64.neg`: write `0 i64 x i64.sub`.

## 8. Control flow

A quotation `[ ... ]` written directly before a combinator is inlined into the
word.

| Form | Rule |
|---|---|
| `cond [ then ] [ else ] if` | both branches start from the same stack and must leave the same stack |
| `cond [ body ] when` / `unless` | body must leave the stack unchanged |
| `[ cond ] [ body ] while` | cond leaves the stack plus one `i32`; body leaves it unchanged; loops while cond is non-zero |
| `[ body ] [ cond ] until` | runs body, then cond; repeats until cond is non-zero |
| `n [ body ] times` | body receives the index (0 to n-1) on top and must consume it |
| `leave` | exits the innermost `while`/`until`/`times`/`each`/`fold`; the stack must match the loop's exit shape |
| `"message" trap` | stops the program with a message |

`leave` and `trap` end their quotation: code after them is an error
(`E_UNREACHABLE`). A branch that ends in `leave` or `trap` does not have to
match the other branch.

## 9. Strings

| Word | Effect | Notes |
|---|---|---|
| `str.len` | `( str -- i32 )` | bytes |
| `str.byte-at` | `( str i32 -- i32 )` | traps out of range |
| `str.slice` | `( str i32 i32 -- str )` | start, length in bytes; traps unless in range and on codepoint boundaries; no copy |
| `str.eq` | `( str str -- i32 )` | |
| `str.concat` | `( str str -- str )` | allocates |
| `str.cp-at` | `( str i32 -- i32 i32 )` | codepoint and its byte length at an offset |
| `str.boundary?` | `( str i32 -- i32 )` | is the offset a codepoint boundary |
| `i32.to-str` `i64.to-str` | `( iNN -- str )` | decimal |
| `f64.fixed` | `( f64 i32 -- str )` | rounded to that many decimals (ties to even); traps beyond the `i64` range |
| `str.from-byte` | `( i32 -- str )` | a one-byte string |
| `str.addr` | `( str -- i32 )` | low level: the address |
| `str.from-raw` | `( i32 i32 -- str )` | low level: unchecked addr and length |
| `mem.alloc` | `( i32 -- i32 )` | low level: zeroed bytes, 8-aligned, never freed |

## 10. Arrays

| Word | Effect |
|---|---|
| `array.new` | `( i32 -- array T )` zeroed; fix `T` with an assertion: `n array.new ( array i32 )` |
| `array.len` | `( array T -- i32 )` |
| `array.at` | `( array T i32 -- T )` bounds-checked |
| `array.at!` | `( array T i32 T -- )` bounds-checked |
| `array.slice` | `( array T i32 i32 -- array T )` start, count; no copy |
| `each` | `arr [ T -- ] each` |
| `map` | `arr [ T -- U ] map` → `array U` |
| `filter` | `arr [ T -- i32 ] filter` → `array T` |
| `fold` | `arr init [ U T -- U ] fold` → `U` |
| `array.to-str` | `( array i32 -- str )` elements in decimal, space-separated |

```
: sum ( array i32 -- i32 )  0 [ i32.add ] fold ;
```

## 10a. Structs

```
struct point  x: i32  y: f64
```

A top-level form: the name, then `field: type` pairs for as long as the next
token is a field label. There is no terminator. A field may have any type,
including a struct declared above, the struct itself, or an `array` of it.
The declaration generates ordinary words, listed by `chasm words` and seen by
`deps` and `used-by`:

| Word | Effect |
|---|---|
| `point.new` | `( i32 f64 -- point )` fields in order |
| `point.x` | `( point -- i32 )` read |
| `point.x!` | `( point i32 -- )` write |

- A struct value is a reference to a record the engine garbage-collects;
  there is no `free`. Every field is mutable, and `!` means write, as for
  locals.
- A struct name is a type anywhere a type is written: effects, assertions,
  fields, `array point`.
- Declaring a struct again with the same fields does nothing; changing its
  fields is `E_REDEFINE_EFFECT`, listing the words that use it.
- A struct can be named only after its declaration, so two structs cannot
  refer to each other (`E_UNKNOWN_TYPE`). The field names `new` and names
  ending in `!` are reserved.
- Struct values cannot be test literals; test a field instead:
  `test point.x : 3 4.5 point.new point.x -> 3`.

**Arrays of structs.** `array point` works with every array word and
combinator; `map` may turn an `array i32` into an `array point` and back.
`array.new` fills it with unset elements, and reading a field of one traps
(`null reference`). Like every array it is a view: `array.slice` shares
storage, and a write through the slice is visible in the original.

**Optional links.** There is no null test. A link that may be absent is an
`array` holding 0 or 1 elements, tested with `array.len`:

```
struct node  v: i32  next: array node

: sum ( array node -- i32 )
  :> l!
  0 :> s!
  [ l array.len ] [ l 0 array.at :> n  s n node.v i32.add s!  n node.next l! ] while
  s ;
```

## 11. Functions as values

```
: twice ( i32 [ i32 -- i32 ] -- i32 )  :> f  f call f call ;
: inc ( i32 -- i32 )  1 i32.add ;
test twice : 5 'inc twice -> 7
```

- `'name` pushes a word's address, typed `[ effect of name ]`.
- `[ body ]` not before a combinator is an anonymous word. It takes no inputs:
  its effect is `( -- outputs )`, worked out from the body. It may not use
  locals of the enclosing word.
- `call` calls the function value on top with the inputs below it.

## 12. Stack assertions

A parenthesised list of types without `--` inside a body asserts the
**whole** stack at that point, top rightmost:

```
: hypot ( f64 f64 -- f64 )
  dup f64.mul      ( f64 f64 )
  swap dup f64.mul ( f64 f64 )
  f64.add f64.sqrt ;
```

`( )` asserts an empty stack. Assertions also fix unknown element types.

## 13. Contracts and tests

```
declare parse-int ( str -- i32 )
test parse-int : "1234" parse-int -> 1234

: double-parsed ( str -- i32 )  parse-int 2 i32.mul ;   # type-checks against the declaration
```

- A declared word compiles to a stub that traps with `unresolved word <name>`.
  Callers check, compile and run until they reach it.
- A later definition must match the declared effect exactly
  (`E_DECLARE_MISMATCH`). Redefining a word with the same effect replaces its
  body for every caller; changing an effect is rejected (`E_REDEFINE_EFFECT`)
  and the error lists the dependants.
- `test word : body -> expected` runs `body` on an empty stack and compares
  the result with the expected literals, type by type. The expected part is
  every literal after `->`, up to the next non-literal. A test of a word with
  no body is reported **pending**. Tests may also name primitives. Each test
  runs in a fresh instance with a captured console.
- `chasm unresolved` is the to-do list: work through it one stub at a time.
- `chasm dead` lists the user words that `main` and the `export` words never
  reach, following calls, quotations and `'word`, over every file named.
  Tests do not keep a word alive. Prelude and struct-generated words are not
  listed, and a program with neither `main` nor an `export` word reports
  no roots rather than calling everything dead.

## 14. I/O

Four host words over a namespace of paths:

| Word | Effect |
|---|---|
| `host.open` | `( str i32 -- i32 )` path, mode → handle or negative error |
| `host.read` | `( i32 i32 i32 -- i32 )` handle, buffer, length → bytes, 0 at end, or error |
| `host.write` | `( i32 i32 i32 -- i32 )` |
| `host.close` | `( i32 -- i32 )` |

Modes: 0 read, 1 write (truncate), 2 append, 3 read-write. Errors: -1 not
found, -2 permission, -3 not supported, -4 I/O error, -5 bad handle; treat any
negative as failure.

| Path | |
|---|---|
| `/dev/cons` | console (stdin and stdout) |
| `/dev/time` | read gives 8 bytes: little-endian `u64` nanoseconds since the Unix epoch |
| `/file/<path>` | host file `/<path>`; a directory read gives directory records |
| `/mnt/<name>/...` | a directory mounted with `--mount name=DIR` |

Directory records: `u32` name length, name bytes, `u64` size, `u8` is-dir,
packed. Reads return whole records only.

Library words:

| Word | Effect |
|---|---|
| `print` / `println` | `( str -- )` |
| `read-line` | `( -- str i32 )` line without newline; flag 0 at end of input |
| `read-file` | `( str -- str i32 )` path → contents, status (0 or error) |
| `ls` | `( str -- i32 )` prints a directory, one entry per line |
| `now` | `( -- i64 )` nanoseconds since the epoch |

## 15. Diagnostic codes

| Code | Meaning |
|---|---|
| `E_LEX`, `E_SYNTAX` | malformed source |
| `E_LITERAL_RANGE` | a literal does not fit its type |
| `E_UNKNOWN_TYPE` | not a type name, or a struct used before its declaration |
| `E_UNDEFINED` | unknown word (the message suggests the nearest name, or the Chasm word for a common name from another language: `pop` → `drop`, `+` → `i32.add`), or used before it is defined or declared |
| `E_STACK_UNDERFLOW` | not enough values; `expected` and `actual` are given |
| `E_TYPE_MISMATCH` | wrong types on top of the stack |
| `E_EFFECT_MISMATCH` | body does not leave the declared outputs |
| `E_ASSERTION` | stack assertion failed |
| `E_BRANCH_MISMATCH` | `if` branches disagree, or a `when` body changes the stack |
| `E_LOOP_EFFECT` | loop body or condition has the wrong shape |
| `E_LEAVE` | `leave` outside a loop or with the wrong stack |
| `E_UNREACHABLE` | code after `leave` or `trap` |
| `E_LOCAL` | local bound twice, assigned while immutable, or named like a primitive |
| `E_CAPTURE` | a quotation value uses a local (no closures) |
| `E_AMBIGUOUS_TYPE` | an element type is never fixed |
| `E_DECLARE_MISMATCH` | definition or redeclaration differs from the declaration |
| `E_REDEFINE_EFFECT` | redefinition changes an effect, redefines a primitive, or changes a struct's fields; lists `dependants` |
| `E_TEST_TYPE` | a test's expected literals do not match what its body leaves |
| `E_MAIN_EFFECT` | `main` is not `( -- )` |
| `E_NO_MAIN` | `run` without `main` |
| `E_IO`, `E_USAGE` | CLI problems |
| `E_INTERNAL` | compiler bug |
| `E_FORGET` | `)forget` refused: the word is still used (lists `dependants`), or is a primitive, prelude or struct-generated word |

## 16. Worked examples

See `examples/`: `hello`, `basics` (words, loops, tests), `strings`,
`arrays` (combinators, functions as values), `contract` (declare first),
`files` (the namespace), `structs` (structs, lists, arrays of structs). Any of them can also be typed or piped into
`chasm repl`, e.g. `chasm repl < examples/basics.chasm`.
