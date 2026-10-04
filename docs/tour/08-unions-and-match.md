# Unions and match

A struct holds all of its fields at once. A union value is exactly one of
several variants, each with fields of its own. It is declared at the top
level: the name, then a `|` before each variant.

```chasm
union shape
  | circle  r: f64
  | rect    w: f64  h: f64
  | empty

: area ( shape -- f64 )
  circle: [ :> r  r r f64.mul 3.14 f64.mul ]
  rect:   [ f64.mul ]
  empty:  [ 0.0 ]
  match ;

: corners ( shape -- i32 )  rect: [ 2drop 4 ] else: [ drop 0 ] match ;

test area : 2.0 3.0 shape.rect area -> 6.0
test area : 1.0 shape.circle area -> 3.14
test area : shape.empty area -> 0.0
test corners : 2.0 3.0 shape.rect corners -> 4
test corners : shape.empty corners -> 0
```

The declaration generates a constructor for each variant, taking its
fields in order: `shape.circle ( f64 -- shape )`,
`shape.rect ( f64 f64 -- shape )` and `shape.empty ( -- shape )`.

## match

`match` takes the union value on top of the stack and runs one labelled
arm, chosen by the variant. The arms are written directly before `match`,
in any order, each as a label and a block.

An arm receives its variant's fields in place of the value, in declaration
order with the last on top. So the `rect:` arm starts with the width and
the height on the stack and `f64.mul` is all it needs, and the `empty:`
arm starts with nothing. An `else:` arm covers every variant not named,
and receives the whole value: in `corners` it drops it.

Like the branches of `if`, every arm must leave the same stack. The
checker also insists that nothing is forgotten. A variant with no arm, and
no `else:`, is `E_MATCH_MISSING`, and the error lists the variants left
out. An arm for something that is not a variant is `E_MATCH_ARM`. Add a
variant to `shape` later and every `match` that needs a new arm is pointed
out before the program runs.

## Looking inside without match

Two more kinds of word are generated. `shape.tag` leaves the variant's
index in declaration order, and each field has a reader, such as
`shape.circle.r`, which traps when the value is some other variant. There
are no writers: a union's fields cannot be changed, so build a new value
instead.

```chasm
union shape
  | circle  r: f64
  | rect    w: f64  h: f64
  | empty

: grow ( shape -- shape )  shape.circle.r 2.0 f64.mul shape.circle ;

test shape.tag : 2.0 3.0 shape.rect shape.tag -> 1
test grow : 1.5 shape.circle grow shape.circle.r -> 3.0
```

## Maybe

The prelude declares one union for you: `option T` is either `none` or
`some` with a value `v`. It is how Chasm says "maybe", in place of a null.
`option.none` and `option.some` build one, and `none:` and `some:` match
it.

```chasm
: or-zero ( option i32 -- i32 )  none: [ 0 ] some: [ ] match ;

test or-zero : 5 option.some or-zero -> 5
test or-zero : option.none or-zero -> 0
```

The `some:` arm is empty because the value it receives is already the
answer.

Next: [Generic words and types](09-generics.md)
