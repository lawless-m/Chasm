# Locals and control flow

## Shuffles, then names

Besides `dup`, `swap` and `drop` there are a few more shuffle words, each
with an effect like any other word: `over ( a b -- a b a )`,
`nip ( a b -- b )`, `rot ( a b c -- b c a )` and
`2dup ( a b -- a b a b )`.

Shuffling stays readable up to about three values. Beyond that, give the
values names. `:> n` pops the top of the stack into a local called `n`,
and writing `n` pushes it again. A local is immutable unless its name is
bound with a `!`: after `:> acc!`, the word `acc!` pops a new value into
it.

```chasm
: sum-to ( i32 -- i32 )
  :> n            # pop into immutable local n
  0 :> acc!       # pop into mutable local acc
  n [ acc i32.add acc! ] times
  acc ;

test sum-to : 5 sum-to -> 10
```

`n [ body ] times` runs the body `n` times. Each time the body receives
the index on top of the stack, 0 to n-1, and must use it up: here it is
added to `acc`, so `5 sum-to` is 0+1+2+3+4.

## Choosing

There is no boolean type: a condition is an `i32`, and 0 is false. The
comparison words leave 1 or 0. `cond [ then ] [ else ] if` runs one of two
blocks, and both must leave the same stack, or the checker reports
`E_BRANCH_MISMATCH`.

```chasm
: fizz ( i32 -- str )
  :> n
  n 15 i32.rem_s i32.eqz [ "FizzBuzz" ] [
    n 3 i32.rem_s i32.eqz [ "Fizz" ] [
      n 5 i32.rem_s i32.eqz [ "Buzz" ] [ n i32.to-str ] if
    ] if
  ] if ;

test fizz : 3 fizz -> "Fizz"
test fizz : 5 fizz -> "Buzz"
test fizz : 15 fizz -> "FizzBuzz"
test fizz : 7 fizz -> "7"

: main ( -- )  15 [ 1 i32.add fizz println ] times ;
```

Locals are visible inside the blocks of `if`, `times` and the other
combinators, which is how `n` reaches the innermost branch; a block passed
around as a value (the next page) cannot see them. With only one branch,
use `cond [ body ] when`, or `unless` for the opposite; the body must
leave the stack as it found it.

## Looping

`[ cond ] [ body ] while` runs the condition, which leaves an `i32` on
top, and repeats the body while that is non-zero. `[ body ] [ cond ] until`
runs the body first and stops once the condition is non-zero.

```chasm
: gcd ( i32 i32 -- i32 )
  [ dup ] [ tuck i32.rem_s ] while
  drop ;

: first-over ( i32 -- i32 )      # the first number whose square exceeds the limit
  :> limit
  0 :> found!
  100 [ :> i  i i i32.mul limit i32.gt_s [ i found! leave ] when ] times
  found ;

: safe-div ( i32 i32 -- i32 )
  dup i32.eqz [ "division by zero" trap ] when
  i32.div_s ;

test gcd : 48 18 gcd -> 6
test first-over : 50 first-over -> 8
test safe-div : 12 4 safe-div -> 3
```

`leave` exits the innermost loop at once. `"message" trap` stops the whole
program with that message: `1 0 safe-div` ends with
``trap in `safe-div`: division by zero``.

Next: [Functions as values](05-functions-as-values.md)
