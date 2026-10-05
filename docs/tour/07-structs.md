# Structs

A struct is a record with named fields. It is declared at the top level:
the name, then `field: type` pairs. There is no terminator; the
declaration ends where the fields do.

```wack
struct point  x: i32  y: f64

: point.add ( point point -- point )
  :> b :> a
  a point.x b point.x i32.add
  a point.y b point.y f64.add
  point.new ;

: nudge ( point -- point )  dup dup point.x 1 i32.add point.x! ;

test point.x : 3 4.5 point.new point.x -> 3
test point.add : 1 2.0 point.new 10 0.5 point.new point.add point.x -> 11
test point.add : 1 2.0 point.new 10 0.5 point.new point.add point.y -> 2.5
test nudge : 3 4.5 point.new nudge point.x -> 4
```

The declaration generates ordinary words. `point.new ( i32 f64 -- point )`
takes the fields in order and leaves a new point. Each field gets a
reader, `point.x ( point -- i32 )`, and a writer,
`point.x! ( point i32 -- )`. Every field is mutable, and `!` means write,
as it does for locals.

From then on `point` is a type, usable wherever a type is written. A
struct value is a reference to a record that the engine garbage-collects
(a WasmGC struct), so there is nothing to free, and `dup` copies the
reference, not the record: `nudge` changes the point it was given.

A struct value cannot be written as a test literal, so a test reads a
field instead, as the tests above do.

## Structs in structs and arrays

A field can be another struct, as long as that struct is declared above.
Arrays hold structs too, and every array word and combinator works on an
`array point`.

```wack
struct point  x: i32  y: f64
struct seg  a: point  b: point

: width ( seg -- i32 )  :> s  s seg.b point.x  s seg.a point.x  i32.sub ;

: points ( i32 -- array point )
  :> n
  n array.new ( array point ) :> a
  n [ :> i  a i  i 0.0 point.new  array.at! ] times
  a ;

: total-x ( array point -- i32 )  0 [ point.x i32.add ] fold ;

test width : 1 0.0 point.new 5 0.0 point.new seg.new width -> 4
test total-x : 4 points total-x -> 6
test map : 4 points [ point.x 10 i32.mul ] map array.to-str -> "0 10 20 30"
```

`array.new` cannot invent points, so a new `array point` holds unset
elements until you store some. Reading a field of an unset element traps
with `null reference`.

## Optional links

A struct can refer to itself, which is how a list is made. There is no
null to test for. A link that may be absent has the type `option node`,
and is either `option.none` or a node wrapped by `option.some`.

```wack
struct node  v: i32  next: option node

: cons ( i32 option node -- option node )  node.new option.some ;

: sum ( option node -- i32 )
  none: [ 0 ] some: [ :> n  n node.v  n node.next sum  i32.add ] match ;

test sum : 1 2 3 option.none cons cons cons sum -> 6
test sum : option.none sum -> 0
```

`match` runs the block labelled with the case it finds: `none:` for the
end of the list, `some:` with the node on the stack. The next page
explains it properly.

Next: [Unions and match](08-unions-and-match.md)
