# The REPL and the CLI

## The REPL

`wack repl` reads one chunk at a time, compiling and running each at
once. A chunk that starts with `:`, `export`, `declare`, `test`, `struct`
or `union` is processed exactly as in a file. Anything else is a line: it
runs on the current stack. After each chunk the stack is shown as
`( types ) values`, bottom to top.

```wack-repl
> : sq ( i32 -- i32 )  dup i32.mul ;
ok: sq ( i32 -- i32 )
( )
> : quad ( i32 -- i32 )  sq sq ;
ok: quad ( i32 -- i32 )
( )
> 3 quad
( i32 ) 81
> : sq ( i32 -- i32 )  dup i32.add ;
ok: sq ( i32 -- i32 )
( i32 ) 81
> drop 3 quad
( i32 ) 12
> : sq ( i32 -- i64 )  i64 ;
<repl:6>:1:3: error[E_REDEFINE_EFFECT]: `sq` is defined ( i32 -- i32 ) but this definition has effect ( i32 -- i64 ); an effect can only change deliberately
    dependants: quad
    declared: ( i32 -- i32 )
( i32 ) 12
> 1 0 i32.div_s
trap in `[line 7]`: wasm trap: integer divide by zero
( i32 ) 12
```

Redefining a word with the same effect takes hold for every caller: after
the new `sq`, `quad` gives 12 without being touched. A different effect is
refused, and the error lists the words that depend on the old one. A line
that traps leaves the stack as it was.

A line starting with `)` is a command to the REPL, not Whackford. `)forget sq`
removes a word, once nothing uses it. `)force` changes an effect
deliberately, rechecking every dependant; the
[reference](../reference.md#1a-the-repl) describes it.

## In the browser

The REPL page of this site runs the same compiler, built for WebAssembly.
It keeps your program in the browser: a reload brings your words back,
`)program` lists what is saved and `)clear` forgets it. A Try-it link
loads its example without saving it. Nothing calls `main` for you: type
`main` to run it.

```wack
: sq ( i32 -- i32 )  dup i32.mul ;
: quad ( i32 -- i32 )  sq sq ;
test quad : 3 quad -> 81

: main ( -- )  3 quad i32.to-str println ;
```

## The command line

```
wack check  FILE...          types and effects only; fast
wack run    FILE...          build and run main
wack test   FILE...          run the test lines
wack unresolved FILE...      declared words with no body yet
wack words  FILE...          every word and its effect
wack prims                   every primitive and its effect
wack deps WORD FILE...       what WORD calls
wack used-by WORD FILE...    what calls WORD
wack dead   FILE...          words main never reaches
wack infer  FILE...          the effects of words that left theirs out
wack build  FILE... -o out.wasm   a WebAssembly module (--wasi for WASI)
wack repl                    the REPL
wack lsp                     a language server for editors
```

A program is the files you name, in order, so a library goes first:
`wack test lib.wack prog.wack`. Add `--json` to any command and it
prints a machine-readable report in place of the text.

## Where next

- The [reference](../reference.md): every form, primitive and diagnostic.
- The Words page of this site: each word with its effect.
- The [examples](https://github.com/lawless-m/Whackford/tree/main/examples) on GitHub: complete programs, each with tests.
