# Functions as values

## Blocks before combinators

`if`, `times` and `while` take their blocks in square brackets. The array
combinators work the same way: `each` runs a block on every element, `map`
builds a new array from the block's results, `filter` keeps the elements
for which the block leaves non-zero, and `fold` threads a running value
through.

```wack
: iota ( i32 -- array i32 )
  :> n
  n array.new ( array i32 ) :> a
  n [ :> i  a i i array.at! ] times
  a ;

: sum ( array i32 -- i32 )  0 [ i32.add ] fold ;
: doubled ( array i32 -- array i32 )  [ 2 i32.mul ] map ;
: evens ( array i32 -- array i32 )  [ 2 i32.rem_u i32.eqz ] filter ;
: scaled ( array i32 i32 -- array i32 )  :> k  [ k i32.mul ] map ;

test sum : 10 iota sum -> 45
test doubled : 5 iota doubled array.to-str -> "0 2 4 6 8"
test evens : 10 iota evens array.to-str -> "0 2 4 6 8"
test scaled : 4 iota 10 scaled array.to-str -> "0 10 20 30"

: main ( -- )  5 iota doubled [ i32.to-str println ] each ;
```

A block written directly before a combinator is inlined into the word: it
is part of the body, not a separate function. That is why the block in
`scaled` can use the local `k`.

## Words as values

A function can also be a value on the stack. `'name` pushes the address of
a word, and `call` runs the function value on top, giving it the values
beneath. The type of a function value is its effect in square brackets, so
an effect can ask for one: `twice` takes an `i32` and a `[ i32 -- i32 ]`.

```wack
: twice ( i32 [ i32 -- i32 ] -- i32 )  :> f  f call f call ;
: inc ( i32 -- i32 )  1 i32.add ;
: square ( i32 -- i32 )  dup i32.mul ;

test twice : 5 'inc twice -> 7
test twice : 3 'square twice -> 81
```

The checker knows the type of every function value, so `call` is checked
like any other word: passing `twice` a word with a different effect is a
type error, found before anything runs.

Combine the two ideas to apply a word, chosen by the caller, to every
element:

```wack
: apply-each ( array i32 [ i32 -- i32 ] -- array i32 )  :> f  [ f call ] map ;
: square ( i32 -- i32 )  dup i32.mul ;
: negate ( i32 -- i32 )  0 swap i32.sub ;

: three ( -- array i32 )
  3 array.new ( array i32 ) :> a
  3 [ :> i  a i  i 1 i32.add  array.at! ] times
  a ;

test apply-each : three 'square apply-each array.to-str -> "1 4 9"
test apply-each : three 'negate apply-each array.to-str -> "-1 -2 -3"
```

## No closures

A block that is not directly before a combinator is a value too: an
anonymous word that takes no inputs, so `[ 42 ]` has the type `[ -- i32 ]`.
Such a block cannot use the locals of the word around it. There are no
closures: writing `:> n  [ n ]` to hand `n` to someone else is `E_CAPTURE`.
Pass what the function needs on the stack instead, as `apply-each` does by
calling `f` inside an inlined block.

Next: [Tests and contracts](06-tests-and-contracts.md)
