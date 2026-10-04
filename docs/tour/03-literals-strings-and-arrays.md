# Literals, strings and arrays

## Numbers

A plain integer is an `i32`: `42`, `-7`, or hex, `0xFF`. For a 64-bit
integer, follow it with the word `i64`: `42 i64` is one literal. A number
with a decimal point or an exponent is an `f64`: `1.5`, `2e10`. There is
no `f32` literal; write `1.5 f32.demote_f64`.

The numeric words are the WebAssembly instructions, under their wasm
names and with wasm behaviour: `i32.add`, `f64.sqrt`, and `i32.div_s`,
which is signed division and traps when the divisor is zero. Nothing
converts by itself. Each conversion is a word that says what it does:
`f64.convert_i32_s` makes an `f64` from a signed `i32`, and `i64` after a
value, rather than a literal, widens an `i32`.

```chasm
: to-fahrenheit ( f64 -- f64 )  1.8 f64.mul 32.0 f64.add ;
: mean ( i32 i32 -- f64 )  i32.add f64.convert_i32_s 2.0 f64.div ;
: double-wide ( i32 -- i64 )  i64 2 i64 i64.mul ;

test to-fahrenheit : 100.0 to-fahrenheit -> 212.0
test to-fahrenheit : 37.0 to-fahrenheit 1 f64.fixed -> "98.6"
test mean : 3 4 mean -> 3.5
test double-wide : 2000000000 double-wide i64.to-str -> "4000000000"
```

`f64.fixed` turns an `f64` into a string, rounded to the number of
decimals given. `i32.to-str` and `i64.to-str` do the same for integers.

## Strings

A string literal is `"text"`, with the escapes `\"`, `\\`, `\n`, `\t` and
`\u{...}` for a codepoint by number. A `str` is immutable UTF-8, and
`str.len` counts bytes, not characters: `"café"` is 5 long.

```chasm
: label ( i32 -- str )  i32.to-str " km" str.concat ;

test label : 42 label -> "42 km"
test label : "café" str.len -> 5
test label : "café" 0 3 str.slice -> "caf"
test label : "tea" "tea" str.eq -> 1
```

`str.concat` joins two strings and `str.eq` compares them, leaving 1 or 0.
`str.slice` takes a start and a length, both in bytes, and both ends must
fall on codepoint boundaries: `"café" 0 4 str.slice` would cut the `é` in
half, so it traps.

## Arrays

An array has a fixed length and one element type. `array.new` takes the
length and leaves a zeroed array, but nothing in `5 array.new` says what
the elements are, so a stack assertion fixes the type: `( array i32 )`.

```chasm
: squares ( i32 -- array i32 )
  :> n
  n array.new ( array i32 ) :> a
  n [ :> i  a i  i i i32.mul  array.at! ] times
  a ;

test squares : 5 squares array.len -> 5
test squares : 5 squares 3 array.at -> 9
test squares : 5 squares array.to-str -> "0 1 4 9 16"
```

`:> n` pops the top of the stack into a local name, and `times` runs its
block once for each index; the next page covers both. `array.at` reads an
element and `array.at!` writes one, both bounds-checked. `array.to-str`
shows an `array i32` in decimal.

An array value is a view, not a copy. `array.slice` takes a start and a
count and leaves a view of the same storage, so writing through the slice
changes the original:

```chasm
: shared ( -- i32 )
  4 array.new ( array i32 ) :> a
  a 1 2 array.slice :> middle
  middle 0 100 array.at!
  a 1 array.at ;

test shared : shared -> 100
```

Next: [Locals and control flow](04-locals-and-control-flow.md)
