# Chasm language reference

This is the working reference for writing Chasm. It describes what the
compiler in this repository accepts today. The design documents
(`ARCHITECTURE.md`, `LANGUAGE.md`) say why; this says how.

Chasm is a typed, concatenative language in the Forth and Factor family. A
program is a sequence of **words**. Every word has a **stack effect**, written
or inferred, and the checker verifies each body against it before anything
runs. Words
compile to WebAssembly functions whose types are their effects.

## 1. Running things

```
chasm check  FILE...          # types and effects only; fast
chasm run    FILE...          # build and run `main ( -- )` (--opt: also wasm-opt)
chasm test   FILE...          # run the `test` lines
chasm unresolved FILE...      # declared words with no body yet
chasm words  FILE...          # every word and its effect
chasm deps WORD FILE...       # what WORD calls (--all for transitive)
chasm used-by WORD FILE...    # what calls WORD
chasm dead   FILE...          # words `main` and `export` words never reach
chasm infer  FILE...          # inferred effects of un-annotated words (--write inserts them)
chasm build  FILE... -o out.wasm  # optimised with Binaryen (--no-opt to skip; --wasi: WASI preview1 module)
chasm repl                    # interactive; reads chunks from stdin
chasm lsp                     # language server over stdio (docs/editors.md)
```

A program is the files you name, in order; there is no include form. To use
a library, name it first: `chasm test lib.chasm prog.chasm`. A pipeline
works too, `cat lib.chasm prog.chasm | chasm test /dev/stdin`, but then error
locations count lines in the concatenated text.

`build` and `run` compile the whole program from its roots, `main` and the
`export` words: words they never reach are left out of the module, and a
reachable word that is declared but has no body is refused (`E_UNRESOLVED`).
A program with no root keeps every word. `build` then runs Binaryen's
`wasm-opt -O3` (version 121 or later; `$CHASM_WASM_OPT` names another
binary), which shrinks the module by about a quarter; if it is missing or
fails, the unoptimised module is written with a note. `run` skips that step
unless given `--opt`, because under wasmtime it is as often slower as faster
(`docs/performance.md`).

`build --wasi` maps the four host words onto WASI preview1 in place of the
ring: `/dev/cons` is stdin and stdout, `/dev/time` is `clock_time_get`,
`/file/<path>` is `path_open` of `<path>` under the runtime's first
preopened directory, and anything else is not found; directories cannot be
read. A trap prints its message to stderr. The module exports `_start`,
which calls `main`, and runs under any WASI preview1 runtime, for example
`wasmtime --dir=. out.wasm`. `run` and `test` keep using the ring host.

Add `--json` to any command for a machine-readable report:

```json
{ "schema": 1, "ok": false, "command": "check",
  "diagnostics": [ { "code": "E_EFFECT_MISMATCH", "severity": "error",
                     "message": "...", "word": "f",
                     "location": { "file": "f.chasm", "line": 3, "column": 1, "token": "f" },
                     "expected": ["i32"], "actual": ["i32", "i32"] } ],
  "results": { ... } }
```

What `results` holds:

| Command | `results` fields |
|---|---|
| `check` | `words`, `library_words`, `tests` (counts), `unresolved` (names), `has_main` |
| `build` | `output`, `bytes`, `unoptimised_bytes`, `optimised`, `note`, `wasi` |
| `run` | `output` (captured console), `trap` (`message`, `word`, or null), `optimised`, `note` |
| `test` | `tests` (each `test`, `word`, `status`, `expected`, `actual`, `trap`, `output`, `location`); `summary` (`pass`, `fail`, `pending`) |
| `unresolved` | `unresolved` (each `word`, `declared_effect`, `dependants`, `pending_tests`, `location`) |
| `dead` | `has_roots`; `dead` (each `word`, `effect`, `location`) |
| `infer` | `words` (each `name`, `effect`, `location`); `written` (files changed by `--write`) |
| `words` | `words` (each `name`, `effect`, `inputs`, `outputs`, `resolved`, `failed`, `export`, `library`, `generated`, `generic`, `inferred`, `instance_of`, `location`) |
| `deps` | `word`; `words` (each `word` and `kind`: `call` or `address-taken`) |
| `used-by` | `word`; `words` (names) |
| `repl` | one report per chunk; see section 1a |

`lsp` prints no report: it speaks JSON-RPC (`docs/editors.md`).

`infer --write` inserts each inferred effect after the word's name,
` ( effect )`, and leaves every other byte of the file unchanged; nothing is
written when the program has errors.

A command that cannot start (an unreadable file, a usage error) reports an
empty `results` object with the error in `diagnostics`. The exit status is 0
exactly when `ok` is true.

Codes are stable; messages may change. See section 15 for the list.

`run`, `test` and `repl` take `--mount NAME=DIR` (exposes `DIR` as
`/mnt/NAME`) or `--mount NAME=9p://HOST:PORT` (a 9P2000 file server over
TCP; native only), `--no-file` (hides the host filesystem) and `--no-net` (hides
`/net`). `repl` also takes
`--json` (one report per chunk, one per line) and `--no-prelude`.

## 1a. The REPL

`chasm repl` reads chunks from stdin and compiles and runs each at once.

- A chunk that starts with `:`, `export`, `declare`, `test`, `struct` or
  `union` is processed
  exactly as in a file. Anything else is a **line**: it runs on the current
  stack, whose types are always known, and its effect is worked out from
  that stack. A line cannot follow a definition in the same chunk.
- A chunk continues on the next line while a `:` definition or a `[`
  quotation is open.
- After each chunk the stack is printed as `( types ) values`, bottom to
  top, or `( )` when empty. Arrays print as `<n elements>`, function
  values as `#slot`, structs with their fields, `point{x: 7, y: 2.5}`,
  union values as their variant, `shape.circle{r: 1.5}` or `shape.empty{}`
  (nested values to three levels, then `seg{...}`), and an unset struct
  as `null`. A value of an instantiated generic type shows under its applied
  type: `option i32 option.some{v: 3}`. A `vec` or `map` echoes as the
  struct it is: `vec{chunks: <32 elements>, count: 1}`. A stack holding a struct or union
  value prints one entry per line, type then value, between `(` and `)`:

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
  holds one: `)forget word`, `)force` and `)words`. `)forget word` removes a word and its tests and frees the name,
  which can then be defined with any effect. It is refused (`E_FORGET`, with
  the `dependants`) while another word, a quotation in one, or another
  word's test uses it: forget those first, top-down. Primitives, prelude
  words and words a `struct` or `union` generated cannot be forgotten. A function value of
  a forgotten word already on the stack still runs the old code. Forgetting
  a generic word removes its instances too.
- `)words` prints the program as it stands, ready to save as a file: each
  `struct` and `union`, each `declare` of a word still defined, the latest
  definition of each word (forgotten and replaced ones left out, every word
  after the words it uses), then the tests still in force, each as typed.
  Text after an item, up to the next one in the same chunk, goes with it.
- A definition without an effect shows its inferred one: `ok: cube ( i32 -- i32 ) (inferred)`.
- `)force` changes a word's effect deliberately. It is followed by one or
  more definitions, and optionally `test` lines; the chunk continues until
  an empty line. Every dependant of a changed word (a word calling it, a
  quotation in one, another word's test; REPL lines do not count) is checked
  again against the new effect, using any new body the chunk gives it. If
  anything fails to check, nothing changes: `E_FORCE` lists the broken
  dependants, followed by their errors. Otherwise everything is committed at
  once, callers are rebuilt and the affected tests run again; the report
  shows `forced: name old -> new` and `rechecked:` for the dependants checked
  from their existing source. The word's own tests go, since they tested the
  old effect: write new ones in the chunk. A function value of the word
  taken before the force still runs the old code at the old type; `'word`
  afterwards is the new one. A definition without an effect in a `)force`
  chunk is inferred first, and a generic word's instances are retired with
  it. Only existing user words can be forced, not
  primitives, prelude words or struct- or union-generated words, and the
  chunk may not hold `declare`, `struct` or `union`.
- `print` writes to the terminal and `read-line` reads from the same stdin
  as the REPL. With `--json` the program's output is captured into
  `results.output`; each report also has `results.defined` (each with
  `inferred`), `results.forgotten`,
  `results.forced`, `results.rechecked`, `results.listing` (`)words`; otherwise null),
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
> : cube dup dup i32.mul i32.mul ;
ok: cube ( i32 -- i32 ) (inferred)
( i32 ) 6561
```

Changing an effect with `)force`:

```
> : f ( -- i32 ) 1 ;
ok: f ( -- i32 )
( )
> : h ( -- i32 ) f ;
ok: h ( -- i32 )
( )
> )force : f ( -- i64 ) 1 i64 ;
.
<repl:2>:1:3: error[E_FORCE]: forcing `f` breaks h; nothing was changed
    dependants: h
    declared: ( -- i32 )
<repl:2>:1:3: error[E_EFFECT_MISMATCH]: `h` is declared ( -- i32 ) but its body leaves ( i64 )
    expected: ( i32 )
    actual:   ( i64 )
( )
> )force : f ( -- i64 ) 1 i64 ;
. : h ( -- i32 ) f i32.wrap_i64 ;
.
ok: f ( -- i64 )
ok: h ( -- i32 )
forced: f ( -- i32 ) -> ( -- i64 )
( )
> h
( i32 ) 1
```

The same REPL runs in the browser; see `web/README.md`.

## 2. Program structure

A file is a sequence of top-level forms, processed **in order**:

```
: name ( inputs -- outputs )  body ;          # define
: name  body ;                                # define, with the effect inferred
export : name ( inputs -- outputs )  body ;   # define and export from the module
declare name ( inputs -- outputs )            # stub: a contract without a body
test name : body -> expected-literals         # a test of `name`
struct name P...  field: type ...             # a struct (section 10a, 10c)
union name P... | variant  field: type ... | ...   # a union (section 10b, 10c)
```

- A word can call itself, and any word defined or declared **above** it.
  To call a word defined further down, `declare` it first.
- `#` starts a comment to end of line. Parentheses are never comments.
- Tokens are separated by whitespace: `[ dup ]`, not `[dup]`.
- The program's entry point is `: main ( -- ) ... ;`.
- A definition may leave out its effect; it is inferred and then checked as
  if written (`chasm infer` shows what was inferred). Words that call
  themselves or each other, `export` words and `main` must write theirs
  (`E_NEEDS_EFFECT`).

## 3. Types and effects

| Type | Meaning | Wasm |
|---|---|---|
| `i32` `i64` `f32` `f64` | numbers; `i32` is also boolean (0 false) and address | itself |
| `str` | immutable UTF-8 bytes | `i32 i32` (addr, byte length) |
| `array T` | mutable, fixed length; `T` not an array | `i32 i32` (addr, count); for a struct `T`, `ref i32 i32` (a view over a WasmGC array: ref, start, count) |
| `name` | a declared struct or union (sections 10a, 10b): a reference to a garbage-collected record | `(ref null $name)` |
| `name T...` | a generic struct or union applied to its type arguments, `pair i32 str` (section 10c) | `(ref null $name)` of that instantiation |
| `[ ins -- outs ]` | a function value | `i32` (table index) |

An effect lists types bottom to top, rightmost on top:
`( str i32 -- i32 i32 )` takes a `str` with an `i32` above it and leaves two
`i32`s.

A name starting with an uppercase letter is a **type variable**: `T`, `U`,
`Elem`, in effects, `array T`, `[ T -- T ]` and stack assertions. A word whose
effect has one is **generic**:

```
: twice ( T -- T T )  dup ;
: first ( array T -- T )  0 array.at ;
```

Each concrete use compiles its own instance, shown as `twice<i32>` by
`words`, `dead` and `deps`. A `T` is any type but matches only itself, so
there are no constraints: `i32.add` on a `T` is a type error. `export` words
and `main` must be concrete; struct and union names are lowercase.

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

`eq` and `hash` also work on any type (section 10d).

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
| `value v1: [ ... ] v2: [ ... ] else: [ ... ] match` | one labelled arm per variant of the union on top, in any order; each receives its variant's fields, `else:` receives the value; every arm leaves the same stack (section 10b) |
| `leave` | exits the innermost `while`/`until`/`times`/`each`/`fold`; the stack must match the loop's exit shape |
| `"message" trap` | stops the program with a message |

`leave` and `trap` end their quotation: code after them is an error
(`E_UNREACHABLE`). A branch or arm that ends in `leave` or `trap` does not
have to match the others. A `match` arm for something that is not a variant
is `E_MATCH_ARM`; a variant without an arm, and no `else:`, is
`E_MATCH_MISSING`.

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
- A struct or union name may be a primitive's name (`map`), but not a type
  keyword (`i32`, `str`, `array`, ...).
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

**Optional links.** There is no null test. A link that may be absent is the
prelude's `option T` (section 10c), matched with `none:` and `some:`:

```
struct node  v: i32  next: option node

: sum ( option node -- i32 )
  none: [ 0 ] some: [ :> n  n node.v  n node.next sum  i32.add ] match ;
```

## 10b. Unions

```
union shape
  | circle  r: f64
  | rect    w: f64  h: f64
  | empty
```

A top-level form: the name, then one or more `| variant` groups, each with
`field: type` pairs as in a struct. There is no terminator. A union value is
exactly one of its variants. The declaration generates ordinary words:

| Word | Effect |
|---|---|
| `shape.circle` | `( f64 -- shape )` a constructor per variant, fields in order |
| `shape.empty` | `( -- shape )` |
| `shape.tag` | `( shape -- i32 )` the variant's index in declaration order |
| `shape.circle.r` | `( shape -- f64 )` a reader per field of each variant; traps (`shape.circle.r: not a circle`) on another variant |

Union fields cannot be written: build a new value instead.

**match.** Labelled arms written directly before `match`, in any order, as
`if` takes its quotations:

```
: area ( shape -- f64 )
  circle: [ :> r  r r f64.mul 3.14 f64.mul ]
  rect:   [ f64.mul ]
  empty:  [ 0.0 ]
  match ;

: corners ( shape -- i32 )  rect: [ 2drop 4 ] else: [ drop 0 ] match ;
```

- An arm receives, in place of the value, its variant's fields in
  declaration order, the last on top. `else:` covers every variant not named
  and receives the whole value.
- Arms are inlined: the word's locals are visible, and `leave` in an arm
  inside a loop works as in `if`.
- Every arm that does not end in `leave` or `trap` leaves the same stack
  (`E_BRANCH_MISMATCH`).
- An arm naming something that is not a variant, a variant twice, `else:`
  twice, or `else:` when every variant is named is `E_MATCH_ARM`. A variant
  without an arm and no `else:` is `E_MATCH_MISSING`, listing the missing
  variants in `expected`.
- `match` on a value that is not a union is `E_TYPE_MISMATCH`. The type of
  the value must be known: `match` as the first thing applied to an input of
  a word without an effect is `E_AMBIGUOUS_TYPE`; write the effect.
- Declaring a union again with the same variants does nothing; changing them
  is `E_REDEFINE_EFFECT`. A name is either a struct or a union, not both.
- Variant names `else` and `tag` are reserved.

## 10c. Generic structs and unions

Type parameters follow the name; a field may use them:

```
struct pair T U  first: T  second: U
union list T | nil | cons  head: T  tail: list T
```

- A generic type is applied to its arguments by position, without
  parentheses: `pair i32 str`, `array pair i32 str`, `pair list i32 str`
  (a `pair` of a `list i32` and a `str`). The wrong number of arguments is
  `E_UNKNOWN_TYPE`.
- The generated words are generic words: `pair.new ( T U -- pair T U )`,
  `pair.first ( pair T U -- T )`, `list.cons ( T list T -- list T )`.
  `3 "x" pair.new` is a `pair i32 str`. A constructor whose arguments do not
  fix every parameter, such as `list.nil`, needs a stack assertion or a
  declared effect (`E_AMBIGUOUS_TYPE`): `list.nil ( list i32 )`.
- Each instantiation is its own WasmGC type, made on first use, and its
  words are instances such as `pair.new<i32,str>` in `words`.
- Generic words work over generic types:
  `: swap-pair ( pair T U -- pair U T )  :> p  p pair.second p pair.first pair.new ;`
- A generic type may name itself in its fields only applied to its own
  parameters in order (`list T` inside `list T`), and a field may use only
  the declared parameters (`E_UNKNOWN_TYPE`).
- Declaring it again with the same parameters and fields does nothing;
  different parameters or fields are `E_REDEFINE_EFFECT`.

The prelude declares `union option T | none | some  v: T`:
`option.none ( -- option T )`, `option.some ( T -- option T )`, matched with
`none:` and `some:`:

```
: or-zero ( option i32 -- i32 )  none: [ 0 ] some: [ ] match ;
test or-zero : 5 option.some or-zero -> 5
```

## 10d. Equality, hashing and collections

**`eq` and `hash`** work on any type, by contents:

| Word | Effect |
|---|---|
| `eq` | `( a a -- i32 )` 1 when equal |
| `hash` | `( a -- i32 )` equal values hash alike |

| Type | Equal when |
|---|---|
| `i32` `i64` | same value |
| `f32` `f64` | same bit pattern: NaN equals itself, `0.0` and `-0.0` differ |
| `str` | same bytes |
| `array T` | same length, elements equal in order |
| a struct | every field equal, recursively |
| a union | same variant, fields equal |
| a function value | same slot |

They work on a `T` inside a generic word, and need the type to be known
(`E_AMBIGUOUS_TYPE`). A cyclic struct value makes them loop forever. The hash
function itself may change between versions.

**`vec T`**, a growable array whose storage never moves:

| Word | Effect |
|---|---|
| `vec.make` | `( -- vec T )` fix `T` with an assertion: `vec.make ( vec i32 )` |
| `vec.push` | `( vec T T -- )` |
| `vec.len` | `( vec T -- i32 )` |
| `vec.at` | `( vec T i32 -- T )` traps out of range |
| `vec.at!` | `( vec T i32 T -- )` traps out of range |
| `vec.pop` | `( vec T -- option T )` |
| `vec.clear` | `( vec T -- )` length 0, storage kept for reuse |
| `vec.to-array` | `( vec T -- array T )` a copy |
| `vec.each` | `( vec T [ T -- ] -- )` |
| `vec.fold` | `( vec T A [ A T -- A ] -- A )` |

```
: add ( i32 i32 -- i32 )  i32.add ;

: squares ( i32 -- vec i32 )
  :> n
  vec.make ( vec i32 ) :> v
  n [ :> i  v i i i32.mul vec.push ] times
  v ;
test vec.at : 4 squares 3 vec.at -> 9
test vec.fold : 4 squares 0 'add vec.fold -> 14
```

`vec.each` and `vec.fold` take function values (`'add`): a quotation value
takes no inputs and cannot use the word's locals. For a loop body that needs
locals, use `vec.to-array [ ... ] each`.

**`map K V`**, a hash map over `hash` and `eq`:

| Word | Effect |
|---|---|
| `map.make` | `( -- map K V )` fix the types: `map.make ( map str i32 )` |
| `map.set` | `( map K V K V -- )` insert or replace |
| `map.get` | `( map K V K -- option V )` |
| `map.has` | `( map K V K -- i32 )` |
| `map.remove` | `( map K V K -- )` |
| `map.len` | `( map K V -- i32 )` |
| `map.keys` | `( map K V -- vec K )` in no particular order |
| `map.each` | `( map K V [ K V -- ] -- )` in no particular order |

```
: or-zero ( option i32 -- i32 )  none: [ 0 ] some: [ ] match ;

: tally ( map str i32 str -- )
  :> w :> m
  m w  m w map.get or-zero 1 i32.add  map.set ;
```

Keys compare by contents, so a struct makes a natural key: a fresh
`3 4 pair.new` finds what was stored under another `3 4 pair.new`. Changing
a key after inserting it loses it.

- **Memory.** Collections live in linear memory, which is never freed: a
  dropped vec or map stays allocated, and a map that grows abandons its old
  arrays. Reuse a vec with `vec.clear` rather than making a fresh one per
  step of a loop.
- The other words named `vec.*`, `chunk.*` and `map.*` (`vec.new`,
  `map.ks`, `map.find`, `map.put`, `map.rehash`, ...) are the implementation.

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
- `'name` on a generic word needs something that fixes its instantiation, a
  stack assertion or a declared effect: `( [ i32 -- i32 i32 ] )`. Otherwise
  it is `E_AMBIGUOUS_TYPE`.

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
Inside a generic word they may name its type variables: `( array T )`.

## 13. Contracts and tests

```
declare parse-int ( str -- i32 )
test parse-int : "1234" parse-int -> 1234

: double-parsed ( str -- i32 )  parse-int 2 i32.mul ;   # type-checks against the declaration
```

- A declared word compiles to a stub that traps with `unresolved word <name>`.
  Callers check, compile and test until they reach it; `build` and `run`
  refuse a stub that `main` or an `export` word can reach.
- A later definition must match the declared effect exactly
  (`E_DECLARE_MISMATCH`). Redefining a word with the same effect replaces its
  body for every caller; changing an effect is rejected (`E_REDEFINE_EFFECT`)
  and the error lists the dependants. Inferred effects follow the same rules.
- A test of a generic word runs on the instance its body uses.
- `chasm words` flags `inferred`, `generic` and `instance of NAME`.
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
found, -2 permission, -3 not supported, -4 I/O error, -5 bad handle, -6
malformed request; treat any negative as failure.

| Path | |
|---|---|
| `/dev/cons` | console (stdin and stdout) |
| `/dev/time` | read gives 8 bytes: little-endian `u64` nanoseconds since the Unix epoch |
| `/file/<path>` | host file `/<path>`; a directory read gives directory records |
| `/mnt/<name>/...` | a directory mounted with `--mount name=DIR`, or a 9p server mounted with `--mount name=9p://host:port`; under a 9p mount, writing to a missing file creates it |
| `/net/http/<host>[:port]/<path>` | an HTTP request; in the browser REPL it goes through `fetch`, so a server on another origin must allow CORS |
| `/net/https/<host>[:port]/<path>` | the same over TLS |

**HTTP requests.** What a program writes to a `/net/http` handle is the rest
of the request after its first line: `Name: value` header lines, an empty
line, then the body. The request is sent at the first read, which returns the
response body; headers may be split across several writes.

- Nothing written, then a read: a GET. `"/net/http/example.com/" read-file`
  is a plain GET.
- `"X-Token: abc\n\n"` written, then a read: a GET with that header.
- `"Content-Type: text/plain\n\nhello"` written, then a read: a POST of
  `hello`.

Status 404 is -1, 401 and 403 are -2, any other status outside 200 to 299 is
-4, and so is a failed connection. A header line without `: `, or a header
block with no empty line after it, is -6; a write after the first read is -2.
`--no-net` hides `/net` (-3).

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

Collections (`vec T`, `map K V`) are in section 10d. The prelude also declares `option T` (section 10c): `option.none ( -- option T )`,
`option.some ( T -- option T )`.

## 15. Diagnostic codes

| Code | Meaning |
|---|---|
| `E_LEX`, `E_SYNTAX` | malformed source |
| `E_LITERAL_RANGE` | a literal does not fit its type |
| `E_UNKNOWN_TYPE` | not a type name, a struct or union used before its declaration, the wrong number of type arguments, a field using an undeclared parameter, or a generic type naming itself with other arguments |
| `E_UNDEFINED` | unknown word (the message suggests the nearest name, or the Chasm word for a common name from another language: `pop` → `drop`, `+` → `i32.add`), or used before it is defined or declared |
| `E_STACK_UNDERFLOW` | not enough values; `expected` and `actual` are given |
| `E_TYPE_MISMATCH` | wrong types on top of the stack |
| `E_EFFECT_MISMATCH` | body does not leave the declared outputs |
| `E_ASSERTION` | stack assertion failed |
| `E_BRANCH_MISMATCH` | `if` branches or `match` arms disagree, or a `when` body changes the stack |
| `E_MATCH_ARM` | a `match` arm names something that is not a variant, a variant twice, `else:` twice, or `else:` when every variant is named |
| `E_MATCH_MISSING` | a variant has no `match` arm and there is no `else:`; `expected` lists the missing variants |
| `E_LOOP_EFFECT` | loop body or condition has the wrong shape |
| `E_LEAVE` | `leave` outside a loop or with the wrong stack |
| `E_UNREACHABLE` | code after `leave` or `trap` |
| `E_LOCAL` | local bound twice, assigned while immutable, or named like a primitive |
| `E_CAPTURE` | a quotation value uses a local (no closures) |
| `E_AMBIGUOUS_TYPE` | an element type is never fixed, a generic word's or constructor's instantiation is not fixed, or `match`, `hash` or `eq` on a value whose type is not known |
| `E_DECLARE_MISMATCH` | definition or redeclaration differs from the declaration |
| `E_REDEFINE_EFFECT` | redefinition changes an effect, redefines a primitive, or changes a struct's fields or a union's variants or a type's parameters; lists `dependants` |
| `E_TEST_TYPE` | a test's expected literals do not match what its body leaves |
| `E_MAIN_EFFECT` | `main` is not `( -- )` |
| `E_NO_MAIN` | `run` without `main` |
| `E_UNRESOLVED` | `build` or `run`: a word `main` or an `export` word reaches is declared but has no body; lists `dependants` |
| `E_IO`, `E_USAGE` | CLI problems |
| `E_INTERNAL` | compiler bug |
| `E_FORGET` | `)forget` refused: the word is still used (lists `dependants`), or is a primitive, prelude, or struct- or union-generated word |
| `E_NEEDS_EFFECT` | the effect must be written: the word is recursive or mutually recursive, exported, or `main`; or an exported word's effect has type variables |
| `E_FORCE` | `)force` refused: a dependant no longer checks (lists `dependants`, then their errors), or the word is a primitive, prelude, or struct- or union-generated word |

## 16. Worked examples

See `examples/`: `hello`, `basics` (words, loops, tests), `strings`,
`arrays` (combinators, functions as values), `contract` (declare first),
`files` (the namespace), `http` (requests with headers), `ninep` (a 9p
mount and directory records), `wasi` (a program for `build --wasi`), `generics` (generic words),
`inferred` (effects left out), `structs` (structs, a list of `option node`
links, arrays of structs), `unions` (shapes with `match` and `else:`, a
recursive `list T`, `option`), `generic-structs` (`pair T U`, a generic word
over it, a struct holding an `option`), `collections` (`vec` push, `at` and
`fold`, a word count with `map str i32`, a map keyed by `pair i32 i32`,
`vec.clear`). Any of them can also be typed or piped into
`chasm repl`, e.g. `chasm repl < examples/basics.chasm`.
