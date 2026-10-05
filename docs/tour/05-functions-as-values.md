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

## Closures

A block that is not directly before a combinator is a value too: a
quotation value, an anonymous function. Its effect comes from its body and
from how it is used, so `[ 2 i32.mul ]` is a `[ i32 -- i32 ]`. It can use
the locals of the word around it: each one it names is captured, by value,
when the block becomes a value. `adder` returns a different function for
every `k`, and each keeps its own.

```wack
: twice ( i32 [ i32 -- i32 ] -- i32 )  :> f  f call f call ;
: adder ( i32 -- [ i32 -- i32 ] )  :> k  [ k i32.add ] ;

test twice : 5 [ 2 i32.mul ] twice -> 20
test twice : 5 3 adder twice -> 11

: halver ( -- [ f64 -- f64 ] )  [ ( f64 -- f64 ) 2.0 f64.div ] ;
test halver : 3.0 halver call -> 1.5

struct counter  n: i32

: make-counter ( -- [ -- i32 ] )
  0 counter.new :> c
  [ c  c counter.n 1 i32.add  counter.n!  c counter.n ] ;
test make-counter : make-counter :> next  next call drop  next call -> 2
```

An effect written straight after `[`, as in `halver`, says what the block
takes and leaves. The checker works it out without one when the block's use
fixes the types, and asks for one (`E_AMBIGUOUS_TYPE`) when nothing does.

Captured values are copies, so a block cannot capture a mutable local
(`:> n!`): that is `E_CAPTURE`. When a function needs state that changes
from call to call, keep it in a struct, as `make-counter` does. A struct is
a reference, so every call sees the same `counter`.

Next: [Tests and contracts](06-tests-and-contracts.md)
