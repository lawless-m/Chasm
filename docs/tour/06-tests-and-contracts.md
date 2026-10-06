# Tests and contracts

## Tests

A test is a line of the program: `test word : body -> expected`. The body
runs on an empty stack, and the whole stack it leaves is compared with the
literals after `->`, type by type. A word that leaves two values is tested
against two, and a test can name a primitive as easily as your own word.

```wack
: div-mod ( i32 i32 -- i32 i32 )  2dup i32.div_s -rot i32.rem_s ;
: shout ( str -- str )  "!" str.concat ;

test div-mod : 17 5 div-mod -> 3 2
test shout : "tea" shout -> "tea!"
test i64.mul : 3 i64 4 i64 i64.mul -> 12 i64
test dup : 1 dup -> 1 1
```

`wack test FILE` runs them. Each test runs in a fresh instance of the
program, so one test cannot disturb the next, and the console is captured,
so a word that prints does not clutter the report.

A word that traps on bad input can be tested for that too: `-> trap`
passes when the body traps, and fails if it returns.

```wack
: digit ( i32 -- i32 )
  48 i32.sub :> d
  d 9 i32.gt_u [ "not a digit" trap ] when
  d ;

test digit : 55 digit -> 7
test digit : 65 digit -> trap
```

## Contract first

You do not need a body to say what a word is for. `declare` gives a name
and an effect, and tests can follow straight away. Other words may call
the declared word and are checked against its effect.

```wack
declare parse-digit ( i32 -- i32 )
test parse-digit : 55 parse-digit -> 7     # 55 is the byte for '7'

: digit-pair ( i32 i32 -- i32 )  parse-digit swap parse-digit 10 i32.mul i32.add ;
```

This program checks. The test of `parse-digit` is reported as pending,
because the word has no body yet, and `wack unresolved` lists what is
still owed:

```
parse-digit ( i32 -- i32 )  used by: digit-pair  pending tests: 1
```

That list is the to-do list: work through it one word at a time. Here is
what `wack test` prints for the program above with two more lines added,
`test i32.add : 2 3 i32.add -> 5` and a deliberately wrong
`test dup : 1 dup -> 1 2`:

```
PENDING  parse-digit #0  (tour.wack:2: word has no body yet)
PASS     i32.add #1
FAIL     dup #2  (tour.wack:7)
    expected: 1 2
    actual:   1 1
1 passed, 1 failed, 1 pending
```

A failing test makes `wack test` exit with an error; a pending one does
not.

## Keeping the promise

The body comes later, anywhere below the declaration, and the pending test
starts to run:

```wack
declare parse-digit ( i32 -- i32 )
test parse-digit : 55 parse-digit -> 7

: digit-pair ( i32 i32 -- i32 )  parse-digit swap parse-digit 10 i32.mul i32.add ;
test digit-pair : 52 50 digit-pair -> 42

: parse-digit ( i32 -- i32 )  48 i32.sub ;

: main ( -- )  52 50 digit-pair i32.to-str println ;
```

The definition must have exactly the declared effect. A body that leaves
an `i64` instead is `E_DECLARE_MISMATCH`: the declaration is a promise to
every caller, and the checker holds you to it.

One more list is worth knowing. `wack dead` names the words that `main`
never reaches, which is what is safe to delete. Tests do not count as a
use, so a word that only its tests call is still listed.

Next: [Structs](07-structs.md)
