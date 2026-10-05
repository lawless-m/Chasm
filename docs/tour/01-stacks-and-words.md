# Stacks and words

Whackford is a stack language. A program is a row of words, read left to
right. Values go on a stack; each word takes some values off the top and
leaves some behind. There are no brackets to match and no precedence to
remember: to add two numbers, put both on the stack, then say `i32.add`.

Here is that in the REPL. After each line it shows the stack, types first,
bottom to top:

```wack-repl
> 3 4 i32.add
( i32 ) 7
> 5 dup
( i32 i32 i32 ) 7 5 5
> swap drop
( i32 i32 ) 7 5
> i32.mul
( i32 ) 35
```

`dup` copies the top value, `swap` exchanges the top two and `drop` throws
the top one away. Because the operator comes after its operands, this
style is called postfix.

## Defining a word

A definition starts with `:`, then the name, then the body, and ends with
`;`. The part in parentheses is the word's stack effect: what it takes,
then `--`, then what it leaves. The next page looks at effects closely.

```wack
: square ( i32 -- i32 )  dup i32.mul ;
test square : 3 square -> 9

: main ( -- )  "7 squared is " print  7 square i32.to-str println ;
```

`square` takes one `i32` and leaves one. The `test` line says that with 3
on the stack, `square` leaves 9; `wack test` runs it. `main ( -- )` is
the program's entry point: it takes nothing and leaves nothing, and
`wack run` calls it. `print` writes a string, `println` adds a newline,
and `i32.to-str` turns a number into a string to print.

Every example on these pages has a Try-it button. It opens the REPL in
your browser with the code already loaded; type `main` there to run it.

## Shuffling

A word is built from other words, and the stack carries the values between
them. `#` starts a comment that runs to the end of the line.

```wack
: double ( i32 -- i32 )  dup i32.add ;       # the value, added to itself
: keep-top ( i32 i32 -- i32 )  swap drop ;   # throw away the one beneath

test double : 21 double -> 42
test keep-top : 1 2 keep-top -> 2
```

Tokens are separated by whitespace and by nothing else, so a name can hold
almost any character: `keep-top`, `i32.to-str`. It also means brackets
need their spaces: write `[ dup ]`, not `[dup]`, which would be read as
one unknown word.

## Strings

Strings go on the stack like numbers. `str.concat` takes two and leaves
them joined.

```wack
: greet ( str -- str )  "Hello, " swap str.concat "!" str.concat ;
test greet : "Ada" greet -> "Hello, Ada!"

: main ( -- )  "world" greet println ;
```

In `greet` the name arrives first, so `swap` puts `"Hello, "` beneath it
before the two are joined.

Next: [Effects and the checker](02-effects-and-the-checker.md)
