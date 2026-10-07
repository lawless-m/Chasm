# Whackford language reference

This is the working reference for writing Whackford. It describes what the
compiler in this repository accepts today. The design documents
(`ARCHITECTURE.md`, `LANGUAGE.md`) say why; this says how.

Whackford is a typed, concatenative language in the Forth and Factor family. A
program is a sequence of **words**. Every word has a **stack effect**, written
or inferred, and the checker verifies each body against it before anything
runs. Words
compile to WebAssembly functions whose types are their effects.

## 1. Running things

```
wack check  FILE...          # types and effects only; fast
wack run    FILE...          # build and run `main ( -- )` (--opt: also wasm-opt)
wack test   FILE...          # run the `test` lines
wack unresolved FILE...      # declared words with no body yet
wack words  FILE...          # every word and its effect
wack prims                   # every primitive and its effect
wack deps WORD FILE...       # what WORD calls (--all for transitive)
wack used-by WORD FILE...    # what calls WORD
wack dead   FILE...          # words `main` and `export` words never reach
wack infer  FILE...          # inferred effects of un-annotated words (--write inserts them)
wack fmt    FILE...          # format in place (--check: change nothing, fail if unformatted)
wack build  FILE... -o out.wasm  # optimised with Binaryen (--no-opt to skip; --wasi: WASI preview1 module)
wack repl                    # interactive; reads chunks from stdin
wack lsp                     # language server over stdio (docs/editors.md)
```

`wack fmt` keeps your line breaks and any gap of two or more spaces (a
phrase break or a lined-up column) and fixes the rest:

- A quotation that spans lines gets `[` and `]` on lines of their own, its
  contents indented one level, and the word after the `]` (the
  combinator) on the next line; a `match` label keeps its `[`
  (`none: [`). A quotation on one line is left alone.
- Items start in column 0, the rest of an item is indented 2, plus 2 for
  each open quotation; comment lines between items go to column 0.
- Single spaces inside `( ... )` and in a definition's header
  (`export raw : name (`); runs of blank lines become one.

```wack fragment
: count ( str -- i32 )
  0 :> n!
  [
    read-line nip
  ]
  [ n 1 i32.add n! ]
  while
  n ;
```

It refuses (`E_INTERNAL`) rather than write a file whose tokens or comments
would change. `examples/` and `bench/` are kept formatted (`cargo test`
checks).

A program is the files you name, in order; there is no include form. To use
a library, name it first: `wack test lib.wack prog.wack`. A pipeline
works too, `cat lib.wack prog.wack | wack test /dev/stdin`, but then error
locations count lines in the concatenated text.

`build` and `run` compile the whole program from its roots, `main` and the
`export` words: words they never reach are left out of the module, and a
reachable word that is declared but has no body is refused (`E_UNRESOLVED`).
A program with no root keeps every word. `build` then runs Binaryen's
`wasm-opt -O3` (version 121 or later; `$WACK_WASM_OPT` names another
binary), which shrinks the module by about a quarter; if it is missing or
fails, the unoptimised module is written with a note. `run` skips that step
unless given `--opt`, because under wasmtime it is as often slower as faster
(`docs/performance.md`).

`run` and `test` keep the machine code wasmtime compiles in
`$XDG_CACHE_HOME/wack` (else `~/.cache/wack`), keyed by the module and the
wasmtime version, so running an unchanged program again skips that compile.
Deleting the directory is always safe.

`build --wasi` maps the four host words onto WASI preview1 in place of the
ring: `/dev/cons` is stdin and stdout, `/dev/time` is `clock_time_get`,
`/file/<path>` is `path_open` of `<path>` under the runtime's first
preopened directory, and anything else is not found; directories cannot be
read. A trap prints its message to stderr. The module exports `_start`,
which calls `main`, and runs under any WASI preview1 runtime, for example
`wasmtime --dir=. out.wasm`. `run` and `test` keep using the ring host.

Programs that use processes (section 14a) run like any other in `run`,
`test` and `repl`. `build` writes the module and notes that its host must
provide the `wack.spawn` and `wack.frames` globals and service ring opcodes
5 to 11; `build --wasi` refuses them (`E_WASI_UNSUPPORTED`).

Add `--json` to any command for a machine-readable report:

```json
{ "schema": 1, "ok": false, "command": "check",
  "diagnostics": [ { "code": "E_EFFECT_MISMATCH", "severity": "error",
                     "message": "...", "word": "f",
                     "location": { "file": "f.wack", "line": 3, "column": 1, "token": "f" },
                     "expected": ["i32"], "actual": ["i32", "i32"] } ],
  "results": { ... } }
```

What `results` holds:

| Command | `results` fields |
|---|---|
| `check` | `words`, `library_words`, `tests` (counts), `unresolved` (names), `has_main` |
| `build` | `output`, `bytes`, `unoptimised_bytes`, `optimised`, `note`, `wasi`, `processes` |
| `run` | `output` (captured console), `trap` (`message`, `word`, `process`, or null), `optimised`, `note` |
| `test` | `tests` (each `test`, `word`, `status`, `expected`, `traps`, `actual`, `trap`, `output`, `location`); `summary` (`pass`, `fail`, `pending`) |
| `unresolved` | `unresolved` (each `word`, `declared_effect`, `dependants`, `pending_tests`, `location`) |
| `dead` | `has_roots`; `dead` (each `word`, `effect`, `location`) |
| `infer` | `words` (each `name`, `effect`, `location`); `written` (files changed by `--write`) |
| `words` | `words` (each `name`, `effect`, `inputs`, `outputs`, `resolved`, `failed`, `export`, `raw`, `library`, `generated`, `generic`, `inferred`, `instance_of`, `location`) |
| `prims` | `primitives` (each `name`, `effect`) |
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

`wack repl` reads chunks from stdin and compiles and runs each at once.

- A chunk that starts with `:`, `export`, `raw`, `declare`, `test`, `struct` or
  `union` is processed
  exactly as in a file. Anything else is a **line**: it runs on the current
  stack, whose types are always known, and its effect is worked out from
  that stack. A line cannot follow a definition in the same chunk.
- A local made with `:>` in a line lasts only to the end of that chunk:
  `4 :> h` then `h` on the next line is `E_UNDEFINED`. What carries from one
  chunk to the next is the stack and the definitions. Keep a value on the
  stack, write the lines that share it as one chunk (in the browser,
  Shift+Enter adds a newline, or ⤢ opens an editor where Ctrl+Enter runs),
  or make it a word: `: h ( -- i32 ) 4 ;`.
- A chunk continues on the next line while a `:` definition or a `[`
  quotation is open.
- After each chunk the stack is printed as `( types ) values`, bottom to
  top, or `( )` when empty. Arrays print as `<n elements>`, function
  values as their type, `[ i32 -- i32 ]`, structs with their fields, `point{x: 7, y: 2.5}`,
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
- A line or a test that uses processes runs as process 0 (section 14a).
- A line starting with `)` is a REPL command, not Whackford, so a file never
  holds one: `)forget word`, `)forget test ...`, `)force`, `)test` and `)words`.
  `)forget test quad : 2 quad -> 9` removes a test: every one in force that
  reads the same, token for token, as `)test` prints it in a `FAIL` line and
  `)words` lists it (spacing and comments aside), or `E_FORGET` if none
  does. `)forget word` removes a word and its tests and frees the name,
  which can then be defined with any effect. It is refused (`E_FORGET`, with
  the `dependants`) while another word, a quotation in one, or another
  word's test uses it: forget those first, top-down. Primitives, prelude
  words and words a `struct` or `union` generated cannot be forgotten. A function value of
  a forgotten word already on the stack still runs the old code. Forgetting
  a generic word removes its instances too.
- `)test` runs every test still in force and prints `PASS`, `FAIL` or
  `PENDING` (the word has no body yet) for each, then
  `N passed, M failed, K pending`. `)test word` runs only the tests of
  `word` and of every word that uses it, directly or through other words,
  quotations or instances: after redefining `word`, the tests its change
  can break. Tests run in the session's shared instance with the console
  not captured, unlike `wack test`.
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
  `results.tests`, `results.trap`, `results.process_traps` (traps in
  spawned processes), `results.stack` and `results.timing`.

```wack-repl
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

```wack-repl
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

```wack fragment
: name ( inputs -- outputs )  body ;          # define
: name  body ;                                # define, with the effect inferred
export : name ( inputs -- outputs )  body ;   # define and export from the module
raw : name ( inputs -- outputs )  body ;      # define, reaching memory by address
declare name ( inputs -- outputs )            # stub: a contract without a body
test name : body -> expected-literals         # a test of `name`
test name : body -> trap                      # a test that `body` traps
struct name P...  field: type ...             # a struct (section 10a, 10c)
union name P... | variant  field: type ... | ...   # a union (section 10b, 10c)
```

- A word can call itself, and any word defined or declared **above** it.
  To call a word defined further down, `declare` it first.
- `#` starts a comment to end of line. Parentheses are never comments.
- `spawn` and `alt` are primitive names, like `match`: they cannot be
  defined or used as locals (section 14a).
- Tokens are separated by whitespace: `[ dup ]`, not `[dup]`.
- The program's entry point is `: main ( -- ) ... ;`.
- `-> trap` expects the body to trap: by `trap`, a check in a prelude word
  or a wasm trap such as dividing by zero, in the test or in a process it
  spawned. The test passes if it traps and fails, showing what the body
  left, if it does not; the other tests run either way. Its results are
  not type-checked. In reports `expected` is `["trap"]` and `traps` is
  true.
- A definition may leave out its effect; it is inferred and then checked as
  if written (`wack infer` shows what was inferred). Words that call
  themselves or each other, `export` words and `main` must write theirs
  (`E_NEEDS_EFFECT`). A word that is declared, and has no body yet, keeps
  its declared effect: `declare w ( str i32 -- i32 )` then `: w ... ;` checks
  the body against it as if written, so an error points into the body. A
  redefinition without an effect keeps the word's effect too when the body
  fits it; one that does not fit is inferred and, its effect differing,
  `E_REDEFINE_EFFECT`.
- **Raw words.** The words that reach memory by address (loads and stores,
  `mem.alloc`, `memory.copy`, `memory.fill`, `str.addr`, `str.from-raw`,
  `bytes.addr`, `bytes.from-raw`), and `ring.submit ( i32 i32 i32 i32 -- i32 )`,
  which submits a ring entry (the prelude's channel words use it), are allowed only in the prelude and in a
  word marked `raw : name ...` (`export raw :` also works); anywhere else,
  tests and REPL lines included, they are `E_RAW`. The checked words
  (`bytes`, `str`, arrays) cover ordinary programs. A `raw` word is called
  like any other, and `wack words` flags it `[raw]`, so every place that
  touches memory directly is easy to find.

## 3. Types and effects

| Type | Meaning | Wasm |
|---|---|---|
| `i32` `i64` `f32` `f64` | numbers; `i32` is also boolean (0 false) and address | itself |
| `str` | immutable UTF-8 bytes | `i32 i32` (addr, byte length) |
| `array T` | mutable, fixed length; `T` not an array | `i32 i32` (addr, count); for a struct `T`, `ref i32 i32` (a view over a WasmGC array: ref, start, count) |
| `name` | a declared struct or union (sections 10a, 10b): a reference to a garbage-collected record | `(ref null $name)` |
| `name T...` | a generic struct or union applied to its type arguments, `pair i32 str` (section 10c) | `(ref null $name)` of that instantiation |
| `[ ins -- outs ]` | a function value | a reference to a closure |

An effect lists types bottom to top, rightmost on top:
`( str i32 -- i32 i32 )` takes a `str` with an `i32` above it and leaves two
`i32`s.

A name starting with an uppercase letter is a **type variable**: `T`, `U`,
`Elem`, in effects, `array T`, `[ T -- T ]` and stack assertions. A word whose
effect has one is **generic**:

```wack
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
| `"text"` | `str`; escapes `\" \\ \n \t \u{1F600}` |
| `"A" char` | `i32`: the codepoint of a one-character string literal, 65 |

There is no `f32` literal: `1.5 f32.demote_f64`.

`char` works only directly after a string literal of one character, and
the two are one literal, so `" " char`, `"\n" char` and `"\"" char` are
32, 10 and 34, and work after `->` in a test. It is the codepoint, which
for ASCII is the byte `str.byte-at` reads; `"é" char` is 233, but `é` is
two bytes in a string (compare with `str.cp-at`). Any other string is
`E_LITERAL_RANGE`; `char` anywhere else is `E_SYNTAX`.

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
| `drop-all` | `( ... -- )` everything on the stack |

`eq` and `hash` also work on any type (section 10d).

`drop-all` clears the REPL stack. In a word it drops the word's own values,
its inputs and what it has pushed, never its caller's: the checker knows
the stack at every point, so it becomes that many drops.

These work on any type; `dup` on a `str` copies both halves.

## 6. Locals

Beyond three or so values, bind names instead of shuffling:

```wack
: sum-to ( i32 -- i32 )
  :> n            # pop into immutable local n
  0 :> acc!       # pop into mutable local acc
  n [ acc i32.add acc! ] times
  acc ;
```

- `:> x` pops into `x`; `:> x!` makes it mutable.
- `x` pushes its value; `x!` pops into it (mutable only).
- Locals are visible inside `[ ]` under combinators (`if`, `times`, ...).
  A quotation value captures the immutable ones it names, by value
  (section 11); naming a mutable local in one is `E_CAPTURE`.
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
- Memory (raw words, section 2): `i32.load` ... `i64.load32_u` take `( i32 -- T )`; stores take `( i32 T -- )` (address below value). Natural alignment, offset 0. `memory.size ( -- i32 )`, `memory.grow ( i32 -- i32 )`, `memory.copy ( dst src n -- )`, `memory.fill ( dst byte n -- )`.

Note there is no `i64.neg`: write `0 i64 x i64.sub`.

## 8. Control flow

A quotation `[ ... ]` written directly before a combinator is inlined into the
word.

| Form | Rule |
|---|---|
| `cond [ then ] [ else ] if` | both branches start from the same stack and must leave the same stack |
| `cond [ body ] when` / `unless` | body must leave the stack unchanged |
| `[ a ] [ b ] and` | runs a; if it leaves 0, that is the answer and b does not run; otherwise b runs and its `i32` is the answer |
| `[ a ] [ b ] or` | runs a; if it leaves non-zero, that is the answer and b does not run; otherwise b runs and its `i32` is the answer |
| `[ cond ] [ body ] while` | cond leaves the stack plus one `i32`; body leaves it unchanged; loops while cond is non-zero |
| `[ body ] [ cond ] until` | runs body, then cond; repeats until cond is non-zero |
| `n [ body ] times` | body receives the index (0 to n-1) on top and must consume it |
| `value v1: [ ... ] v2: [ ... ] else: [ ... ] match` | one labelled arm per variant of the union on top, in any order; each receives its variant's fields, `else:` receives the value; every arm leaves the same stack (section 10b) |
| `leave` | exits the innermost `while`/`until`/`times`/`each`/`fold`; the stack must match the loop's exit shape |
| `"message" trap` | stops the program with a message |

`and` and `or` short-circuit, unlike `i32.and` and `i32.or`, which take two
values already computed. Each quotation starts from the same stack and leaves
it plus one `i32`, so the second can guard a read the first has checked:
`[ i s str.len i32.lt_u ] [ s i str.byte-at 48 i32.eq ] and`. The answer is
the deciding quotation's value, not forced to 0 or 1.

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
| `str.to-i32` `str.to-i64` | `( str -- iNN i32 )` | the whole string as a number, and 1; 0 and 0 unless it is exactly an optional `-` and digits that fit |
| `str.i32-at` `str.i64-at` | `( str i32 -- iNN i32 )` | the number starting at an offset (optional `-`, then digits) and the offset after it; 0 and -1 if none starts there, it does not fit, or the offset is outside the string |
| `f64.fixed` | `( f64 i32 -- str )` | rounded to that many decimals (ties to even); traps beyond the `i64` range |
| `str.from-byte` | `( i32 -- str )` | a one-byte string |
| `str.addr` | `( str -- i32 )` | raw (section 2): the address |
| `str.from-raw` | `( i32 i32 -- str )` | raw: unchecked addr and length |

`str.i32-at` picks numbers out of a line without splitting it: on
`"move 3 from 12 to 2"`, offset 5 gives `3 6` and offset 12 gives `12 14`.
None of the parsing words skips spaces or accepts `+`; none traps.
| `mem.alloc` | `( i32 -- i32 )` | raw: zeroed bytes, 8-aligned, never freed |

## 9a. Byte buffers

A `bytes` is a mutable buffer that carries its length, as a `str` does: it
is what `host.read` fills (section 14), and it cannot be read or written
past its end.

| Word | Effect | Notes |
|---|---|---|
| `bytes.new` | `( i32 -- bytes )` | zeroed; traps on a negative length |
| `bytes.len` | `( bytes -- i32 )` | |
| `bytes.at` | `( bytes i32 -- i32 )` | the byte at an offset; traps out of range |
| `bytes.at!` | `( bytes i32 i32 -- )` | offset, byte; traps out of range |
| `bytes.slice` | `( bytes i32 i32 -- bytes )` | start, length; traps out of range; shares the buffer, no copy |
| `bytes.to-str` | `( bytes -- str )` | a copy |
| `bytes.as-str` | `( bytes -- str )` | the buffer itself, no copy: for a buffer you are finished with, as a later write shows through the string |
| `bytes.put` | `( bytes i32 str -- )` | copy a string in at an offset; traps unless it fits |
| `bytes.u16-at` `bytes.u32-at` `bytes.u64-at` | `( bytes i32 -- i32 )` `( bytes i32 -- i32 )` `( bytes i32 -- i64 )` | little-endian number at a byte offset (u16 is 0 to 65535); traps out of range |
| `bytes.u16-at!` `bytes.u32-at!` `bytes.u64-at!` | `( bytes i32 i32 -- )` `( bytes i32 i32 -- )` `( bytes i32 i64 -- )` | offset, value (the low 16 bits for u16); traps out of range |
| `bytes.addr` | `( bytes -- i32 )` | raw (section 2): the address |
| `bytes.from-raw` | `( i32 i32 -- bytes )` | raw: unchecked addr and length |

`eq` and `hash` compare buffers by contents.

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

```wack
: sum ( array i32 -- i32 )  0 [ i32.add ] fold ;
```

## 10a. Structs

```wack
struct point  x: i32  y: f64
```

A top-level form: the name, then `field: type` pairs for as long as the next
token is a field label. There is no terminator. A field may have any type,
including a struct declared above, the struct itself, or an `array` of it.
The declaration generates ordinary words, listed by `wack words` and seen by
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

```wack
struct node  v: i32  next: option node

: sum ( option node -- i32 )
  none: [ 0 ] some: [ :> n  n node.v  n node.next sum  i32.add ] match ;
```

## 10b. Unions

```wack
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

```wack fragment
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

```wack
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

```wack
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
| a function value | the same value: two `'inc` are different |

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

```wack
: add ( i32 i32 -- i32 )  i32.add ;

: squares ( i32 -- vec i32 )
  :> n
  vec.make ( vec i32 ) :> v
  n [ :> i  v i i i32.mul vec.push ] times
  v ;
test vec.at : 4 squares 3 vec.at -> 9
test vec.fold : 4 squares 0 'add vec.fold -> 14

: add-all ( vec i32 i32 -- vec i32 )
  :> k :> v
  vec.make ( vec i32 ) :> out
  v [ k i32.add  out swap vec.push ] vec.each
  out ;
test add-all : 3 squares 10 add-all vec.to-array array.to-str -> "10 11 14"
```

`vec.each`, `vec.fold` and `map.each` take function values: a `'word`
such as `'add`, or a quotation value, which can use the word's locals
(`k` and `out` in `add-all`).

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

```wack
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

```wack
: twice ( i32 [ i32 -- i32 ] -- i32 )  :> f  f call f call ;
: inc ( i32 -- i32 )  1 i32.add ;
test twice : 5 'inc twice -> 7
test twice : 5 [ 2 i32.mul ] twice -> 20

: adder ( i32 -- [ i32 -- i32 ] )  :> k  [ k i32.add ] ;
test twice : 5 3 adder twice -> 11
```

- `'name` pushes a word as a function value, typed `[ effect of name ]`.
- `[ body ]` not before a combinator is a quotation value, an anonymous
  function. Its effect is inferred from the body, as for a word whose effect
  is left out, with its types fixed by how it is used: `[ 2 i32.mul ]` is
  `[ i32 -- i32 ]`. When nothing fixes them (`[ dup ] drop`), it is
  `E_AMBIGUOUS_TYPE`: write the effect right after `[`, as in
  `[ ( i32 -- i32 i32 ) dup ]`.
- A quotation value captures the immutable locals it names, by value, when
  it is made: `adder` returns a closure holding its `k`. Naming a mutable
  local in one is `E_CAPTURE`; state that changes between calls goes in a
  struct, and the closure captures the struct.
- `call` calls the function value on top with the inputs below it.
- `eq` on function values is 1 only for the same value.
- `'name` on a generic word needs something that fixes its instantiation, a
  stack assertion or a declared effect: `( [ i32 -- i32 i32 ] )`. Otherwise
  it is `E_AMBIGUOUS_TYPE`.

## 12. Stack assertions

A parenthesised list of types without `--` inside a body asserts the
**whole** stack at that point, top rightmost:

```wack
: hypot ( f64 f64 -- f64 )
  dup f64.mul      ( f64 f64 )
  swap dup f64.mul ( f64 f64 )
  f64.add f64.sqrt ;
```

`( )` asserts an empty stack. Assertions also fix unknown element types.
Inside a generic word they may name its type variables: `( array T )`.

## 13. Contracts and tests

```wack
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
- `wack words` flags `inferred`, `generic` and `instance of NAME`.
- `test word : body -> expected` runs `body` on an empty stack and compares
  the result with the expected literals, type by type. The expected part is
  every literal after `->`, up to the next non-literal. A test of a word with
  no body is reported **pending**. Tests may also name primitives. Each test
  runs in a fresh instance with a captured console.
- `wack unresolved` is the to-do list: work through it one stub at a time.
- `wack dead` lists the user words that `main` and the `export` words never
  reach, following calls, quotations and `'word`, over every file named.
  Tests do not keep a word alive. Prelude and struct-generated words are not
  listed, and a program with neither `main` nor an `export` word reports
  no roots rather than calling everything dead.

## 14. I/O

Four host words over a namespace of paths:

| Word | Effect |
|---|---|
| `host.open` | `( str i32 -- i32 )` path, mode → handle or negative error |
| `host.read` | `( i32 bytes -- i32 )` handle, buffer → bytes read (at most the buffer's length), 0 at end, or error |
| `host.write` | `( i32 str -- i32 )` handle, string → bytes written, or error |
| `host.close` | `( i32 -- i32 )` |

Modes, as prelude words: `host.OREAD` (0), `host.OWRITE` (1, truncates),
`host.OAPPEND` (2), `host.ORDWR` (3). Errors: -1 not found, -2
permission, -3 not supported, -4 I/O error, -5 bad handle, -6 malformed
request; treat any negative as failure.

`host.read` fills a `bytes` buffer (section 9a) and never more than its
length; take the part it filled with `bytes.slice` and make a string of it
with `bytes.to-str` (a copy) or `bytes.as-str` (no copy, when the buffer is
not written again). `host.write` takes a string.

```wack
# Print a file through a 64-byte buffer, a chunk at a time. Status is 0
# at the end, or the error.
: cat ( str -- i32 )
  host.OREAD host.open :> h
  h 0 i32.lt_s
  [ h ]
  [
    64 bytes.new :> buf
    0 :> n!
    [ h buf host.read n!  n 0 i32.gt_s ]
    [ buf 0 n bytes.slice bytes.to-str print ]
    while
    h host.close drop
    n
  ]
  if ;

: main ( -- )
  "/dev/cons" host.OWRITE host.open :> out
  out "the hostname: " host.write drop
  "/file/etc/hostname" cat
  0 i32.lt_s [ "could not read it (the browser REPL has no /file)" println ] when ;
```

| Path | |
|---|---|
| `/dev/cons` | console (stdin and stdout) |
| `/dev/time` | read gives 8 bytes: little-endian `u64` nanoseconds since the Unix epoch |
| `/file/<path>` | host file `/<path>`; a directory read gives directory records |
| `/mnt/<name>/...` | a directory mounted with `--mount name=DIR`, or a 9p server mounted with `--mount name=9p://host:port`; under a 9p mount, writing to a missing file creates it |
| `/net/http/<host>[:port]/<path>` | an HTTP request; in the browser REPL it goes through `fetch`, so a server on another origin must allow CORS |
| `/net/https/<host>[:port]/<path>` | the same over TLS |
| `/prog`, `/prog/<pid>/ctl` | reading `/prog` lists the live processes; writing `kill` to a `ctl` ends one (section 14a) |
| `/local/<name>` | browser REPL only: a file kept in the page's local storage, across visits; `<name>` is one path segment; reading `/local` gives directory records |

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
| `host.read-line` | `( i32 -- str i32 )` the same from an open handle, which stays open |
| `read-file` | `( str -- str i32 )` path → contents, status (0 or error) |
| `write-file` | `( str str -- i32 )` contents, path → status; replaces the file |
| `copy` | `( str str -- i32 )` from, to → status: `"/net/https/example.com/" "/local/page" copy` |
| `ls` | `( str -- i32 )` prints a directory, one entry per line |
| `now` | `( -- i64 )` nanoseconds since the epoch |

`host.read-line` reads one byte per `host.read` until a newline or end of
input, so it takes nothing past the newline and the next read carries on
from the next line; the line has no length limit. A last line without a
newline still comes back with flag 1, and the next call gives `""` and 0.
Only the `\n` is removed: a `\r\n` line keeps its `\r`. `read-line` opens
`/dev/cons`, calls `host.read-line` and closes it. At `wack repl` it reads
the same stdin as the REPL; in the browser REPL console reads are end of
input, so it gives `""` and 0 at once.

Collections (`vec T`, `map K V`) are in section 10d. The prelude also declares `option T` (section 10c): `option.none ( -- option T )`,
`option.some ( T -- option T )`.

## 14a. Processes and channels

Processes are cooperative green threads in one instance: one runs at a
time, and they switch only at channel operations, `time.sleep` and when a
process finishes (host I/O completes inline). They run in `wack run`, `wack test`,
`wack repl` and the browser REPL, all by the same mechanism: a process that
must wait unwinds its wasm stack into GC frames, and is rewound when it
resumes; the compiler transforms the words that can be on the stack
when a process waits, so a program without processes compiles exactly as
before. `wack build` writes the transformed module with a note
(`results.processes`); `build --wasi` refuses it (`E_WASI_UNSUPPORTED`).

- `spawn ( [ -- ] -- )` starts a function value as a process. A closure
  reaches only what it captured, so the channels it captures are its whole
  connection to the rest of the program. Spawning does not switch: the new
  process runs when the current one waits or finishes.
- References are shared: a struct, `vec` or `map` sent on a channel or
  captured by two processes is the same value in both, as in Limbo and Go.
  Since processes switch only at channel operations, nothing
  changes it under a running process.
- Output of two processes printing at once interleaves at write
  granularity.

| Word | Effect |
|---|---|
| `chan.make` | `( -- chan T )` a channel with one sender; fix `T`: `chan.make ( chan i32 )` |
| `chan.sender` | `( chan T -- )` one more sender, before the `spawn` that will send |
| `chan.send` | `( chan T T -- )` waits until a receiver takes the value; traps `chan.send: the channel is closed` |
| `chan.recv` | `( chan T -- option T )` waits for a value; `none` once the channel is closed and drained |
| `chan.close` | `( chan T -- )` one sender is done; closed when every sender has closed; traps when closed more often than it has senders |
| `time.sleep` | `( i32 -- )` waits that many milliseconds (a negative count is 0); other processes run meanwhile |
| `time.after` | `( T i32 -- chan T )` given a value and a count, a channel that receives the value once, that many milliseconds from now, and is then closed |

The close is explicit and counted: a reader learns that a channel is
finished from the channel (`chan.recv` gives `none`), never from a special
value. Each sender closes once, so with two producers the channel needs
`chan.sender` and two `chan.close`s:

```wack
: produce ( chan i32 i32 -- )
  :> n :> c
  n [ 1 i32.add c swap chan.send ] times
  c chan.close ;

: sum-of ( i32 -- i32 )
  :> n
  chan.make ( chan i32 ) :> c
  c chan.sender
  [ c n produce ] spawn
  [ c n produce ] spawn
  0 :> total!
  [ 1 ]
  [
    c chan.recv
    none: [ leave ]
    some: [ total i32.add total! ]
    match
  ]
  while
  total ;
test sum-of : 3 sum-of -> 12
```

`alt` waits on several channels and runs the arm of the first with a value,
giving it `option T` (`none` when that channel is closed and drained). Each
arm is one token naming a channel, the label `recv:`, and a block; arms are
tried in the order written, and every arm leaves the same stack
(`E_BRANCH_MISMATCH` otherwise). A closed and drained channel is always
ready, so a loop over `alt` stops waiting on a channel once it has seen its
`none` (see `examples/alt.wack`).

```wack fragment
evens recv: [ none: [ 0 ] some: [ ] match ]
odds recv: [ none: [ 0 ] some: [ ] match ]
alt
```

An `alt` arm on `time.after`'s channel is a timeout, and the value given to
`time.after` is what that arm receives, so `-1 10 time.after` makes -1 the
timeout's answer. `time.after` is a prelude word that spawns a process which
sleeps, sends and closes, so when no one receives (the other arm won) that
process stays parked on its send: a `run` or `test` drops it when process 0
returns, a REPL keeps it and `/prog` lists it. A sleeping process is never
blocked: the all-processes-blocked trap fires only when nothing is ready and
nothing sleeps. A run ends when `main` has returned, even if other processes
are still sleeping. See `examples/timeout.wack`.

```wack
# The value from c, or -1 when ms milliseconds pass first.
: recv-within ( chan i32 i32 -- i32 )
  :> ms :> c
  -1 ms time.after :> timer
  c recv: [ none: [ -1 ] some: [ ] match ]
  timer recv: [ none: [ -1 ] some: [ ] match ]
  alt ;

: slow-producer ( -- i32 )
  chan.make ( chan i32 ) :> c
  [ 50 time.sleep  c 7 chan.send ] spawn
  c 10 recv-within ;
test slow-producer : slow-producer -> -1
```

`main`, a test or a REPL line runs as process 0. `wack run` ends when
`main` has returned and no other process can run; processes still waiting
then are dropped. A trap in a spawned process ends the run (or fails the
test) as ``trap in `word` (process N): message``, and `results.trap.process`
is N (null for process 0).

When no process can run and some are waiting on channels, the program can
never finish: process 0 traps with
`all processes blocked: main waits to receive on chan 2; process 3 waits to send on chan 1; ...`,
naming each waiting process and its channels (`waits on chan 1, chan 2
(alt)` for an `alt`); process 0 is `main`, the test's word or `[line]`.

In the REPL, a line or a test runs as process 0. A step finishes when process 0
has returned and no other process can run; processes still waiting stay, and a later line
can wake them. A line that could only wait for ever traps as blocked, with
the stack unchanged. A trap in a spawned process prints as
``trap in `word` (process N): message`` and ends only that process.

```wack-repl
> : schan ( -- chan str ) chan.make ;
ok: schan ( -- chan str )
( )
> schan :> c  c  [ c chan.recv none: [ "closed" println ] some: [ println ] match ] spawn
(
chan str chan{id: 1, q: vec{chunks: <32 elements>, count: 0}, head: 0}
)
> dup "hello" chan.send
hello
(
chan str chan{id: 1, q: vec{chunks: <32 elements>, count: 0}, head: 0}
)
> drop schan chan.recv
trap in `[line]`: all processes blocked: [line] waits to receive on chan 2
(
chan str chan{id: 1, q: vec{chunks: <32 elements>, count: 0}, head: 0}
)
```

`/prog` lists the live processes by number (`"/prog" ls`), and writing `kill`
to `/prog/<pid>/ctl` ends one (`"kill" "/prog/3/ctl" write-file`); anything
else written there is `-6`. A killed process that is waiting never resumes,
and one that kills itself stops at that write.

## 15. Diagnostic codes

| Code | Meaning |
|---|---|
| `E_LEX`, `E_SYNTAX` | malformed source |
| `E_LITERAL_RANGE` | a literal does not fit its type |
| `E_UNKNOWN_TYPE` | not a type name, a struct or union used before its declaration, the wrong number of type arguments, a field using an undeclared parameter, or a generic type naming itself with other arguments |
| `E_UNDEFINED` | unknown word (the message suggests the nearest name, or the Whackford word for a common name from another language: `pop` → `drop`, `+` → `i32.add`), or used before it is defined or declared |
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
| `E_CAPTURE` | a quotation value uses a mutable local |
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
| `E_RAW` | a word that reaches memory by address (a load or store, `mem.alloc`, `memory.copy`, `memory.fill`, `str.addr`, `str.from-raw`, `bytes.addr`, `bytes.from-raw`) or submits a ring entry (`ring.submit`) outside the prelude and outside a `raw` word |
| `E_WASI_UNSUPPORTED` | `build --wasi`: the program uses processes, which a WASI module cannot run |
| `E_PROCESSES_UNSUPPORTED` | kept but no longer emitted: processes run natively since M12 |
| `E_FORCE` | `)force` refused: a dependant no longer checks (lists `dependants`, then their errors), or the word is a primitive, prelude, or struct- or union-generated word |

## 16. Worked examples

See `examples/`: `hello`, `basics` (words, loops, tests), `strings`,
`arrays` (combinators, functions as values), `contract` (declare first),
`files` (the namespace), `bytes` (byte buffers and reading in chunks), `http` (requests with headers), `ninep` (a 9p
mount and directory records), `wasi` (a program for `build --wasi`), `generics` (generic words),
`inferred` (effects left out), `structs` (structs, a list of `option node`
links, arrays of structs), `unions` (shapes with `match` and `else:`, a
recursive `list T`, `option`), `generic-structs` (`pair T U`, a generic word
over it, a struct holding an `option`), `collections` (`vec` push, `at` and
`fold`, a word count with `map str i32`, a map keyed by `pair i32 i32`,
`vec.clear`), `function-composition` (`compose` returning a closure),
`closures` (capture by value, a counter boxed in a struct, closures given to
`vec.each`). Any of them can also be typed or piped into
`wack repl`, e.g. `wack repl < examples/basics.wack`.

Two examples use processes: `pipeline` (two producers and a counted close)
and `alt` (merging two streams, and a ping-pong).
