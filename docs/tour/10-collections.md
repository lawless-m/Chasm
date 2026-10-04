# Collections

## Equal by contents

Two words work on any type and look at contents, not identity. `eq` leaves
1 when two values are equal: strings with the same bytes, arrays equal
element by element, structs equal field by field, unions with the same
variant and fields. `hash` leaves an `i32` that is the same for equal
values. The two collections in the prelude are built on them.

## vec

An array has a fixed length. A `vec T` grows: `vec.push` adds to the end.
`vec.make` takes no arguments, so an assertion fixes the element type.

```chasm
: add ( i32 i32 -- i32 )  i32.add ;
: or-zero ( option i32 -- i32 )  none: [ 0 ] some: [ ] match ;

: squares ( i32 -- vec i32 )
  :> n
  vec.make ( vec i32 ) :> v
  n [ :> i  v i i i32.mul vec.push ] times
  v ;

test vec.len : 4 squares vec.len -> 4
test vec.at : 4 squares 3 vec.at -> 9
test vec.fold : 4 squares 0 'add vec.fold -> 14
test vec.pop : 4 squares vec.pop or-zero -> 9
test vec.pop : 0 squares vec.pop or-zero -> 0
test vec.to-array : 4 squares vec.to-array array.to-str -> "0 1 4 9"
```

`vec.at` reads an element and traps out of range. `vec.pop` removes the
last element and leaves an `option T`, because the vec may be empty.

`vec.each` and `vec.fold` are ordinary words, not combinators, so they
take a function value such as `'add`. A block handed to them is a value
too, and cannot use the word's locals. When the loop body needs locals,
copy the vec out and use the array combinator: `vec.to-array [ ... ] each`.

## map

A `map K V` is a hash map from keys to values. `map.set` inserts or
replaces, and `map.get` leaves an `option V`, since the key may be absent.

```chasm
: or-zero ( option i32 -- i32 )  none: [ 0 ] some: [ ] match ;

: tally ( map str i32 str -- )
  :> w :> m
  m w  m w map.get or-zero 1 i32.add  map.set ;

: counts ( -- map str i32 )
  map.make ( map str i32 ) :> m
  m "tea" tally  m "milk" tally  m "tea" tally
  m ;

test tally : counts "tea" map.get or-zero -> 2
test tally : counts "sugar" map.get or-zero -> 0
test map.has : counts "milk" map.has -> 1
test map.len : counts map.len -> 2
test map.remove : counts :> m  m "tea" map.remove  m map.len -> 1
test map.keys : counts map.keys vec.len -> 2
```

Keys are compared with `eq`, by contents, so a struct makes a natural key:
a new pair finds what was stored under another with the same fields.

```chasm
struct pair T U  first: T  second: U

: grid ( -- map pair i32 i32 str )
  map.make ( map pair i32 i32 str ) :> g
  g 3 4 pair.new "tree" map.set
  g 0 0 pair.new "start" map.set
  g ;

test eq : 3 4 pair.new 3 4 pair.new eq -> 1
test map.has : grid 3 4 pair.new map.has -> 1
test map.has : grid 4 3 pair.new map.has -> 0
```

## A note on memory

Collections live in linear memory, which is never freed: a vec or map you
drop stays allocated. In a loop, reuse one vec with `vec.clear`, which
sets its length to 0 and keeps its storage, rather than making a new one.

Next: [Input and output](11-io.md)
