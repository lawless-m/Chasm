# Effects and the checker

Every word has a stack effect: the types it takes, then `--`, then the
types it leaves. An effect lists the stack bottom to top, so the rightmost
type is the one on top. `( str i32 -- i32 i32 )` takes a `str` with an
`i32` above it and leaves two `i32`s.

```chasm
: area ( i32 i32 -- i32 )  i32.mul ;
: div-mod ( i32 i32 -- i32 i32 )  2dup i32.div_s -rot i32.rem_s ;

test area : 3 4 area -> 12
test div-mod : 17 5 div-mod -> 3 2
```

`div-mod` leaves two values, the quotient with the remainder above it, and
its test lists them in the same order.

## Checked before anything runs

The checker walks each body, tracking the types on the stack, and compares
what is left with the effect. It does this for the whole program before
any of it runs. Here is a word whose body does not do what its effect
says:

```chasm fragment
: bad ( i32 -- i32 )  dup ;
```

`chasm check` refuses it:

```
bad.chasm:1:3: error[E_EFFECT_MISMATCH]: `bad` is declared ( i32 -- i32 ) but its body leaves ( i32 i32 )
    expected: ( i32 )
    actual:   ( i32 i32 )
```

`chasm run` refuses the program too: nothing runs until every word
checks. The codes,
`E_EFFECT_MISMATCH` here, are stable; the
[reference](../reference.md#15-diagnostic-codes) lists them all.

## Stack assertions

Inside a body, a parenthesised list of types without `--` is an assertion:
it states the whole stack at that point, and the checker verifies it. It
tells the reader where things stand halfway through a longer word.

```chasm
: hypot ( f64 f64 -- f64 )
  dup f64.mul      ( f64 f64 )
  swap dup f64.mul ( f64 f64 )
  f64.add f64.sqrt ;

test hypot : 3.0 4.0 hypot -> 5.0
```

An assertion that does not hold is an error, `E_ASSERTION`, like any other
mismatch.

## Leaving the effect out

A definition may leave out its effect. The checker infers it, and from
then on treats it exactly as if it were written.

```chasm
: cube  dup dup i32.mul i32.mul ;
test cube : 3 cube -> 27

: main ( -- )  3 cube i32.to-str println ;
```

`chasm infer` prints what was inferred: `cube ( i32 -- i32 )`. Three kinds
of word must write their effect: `main`, exported words, and words that
call themselves or each other. Leaving it off one of those is
`E_NEEDS_EFFECT`.

Next: [Literals, strings and arrays](03-literals-strings-and-arrays.md)
