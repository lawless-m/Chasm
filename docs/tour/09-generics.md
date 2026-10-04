# Generic words and types

## Type variables

`dup` works on any type. Your own words can too. In an effect, a name
starting with an uppercase letter is a type variable: it stands for
whatever type the caller has.

```chasm
: twice ( T -- T T )  dup ;
: first ( array T -- T )  0 array.at ;

: nums ( -- array i32 )
  2 array.new ( array i32 ) :> a
  a 0 10 array.at!  a 1 20 array.at!
  a ;

test twice : 3 twice -> 3 3
test twice : "a" twice -> "a" "a"
test first : nums first -> 10
test first : nums [ i32.to-str ] map first -> "10"

: main ( -- )  21 twice i32.add i32.to-str println ;
```

`twice` is used here at `i32` and at `str`. Each concrete use compiles its
own instance, and `chasm words` lists them by name: `twice<i32>`,
`twice<str>`.

A `T` is any type, but inside the word it matches only itself. There are
no constraints to say "any number", so the body may only do what works for
every type: shuffle the value, store it, pass it on. `i32.add` on a `T` is
a type error.

## Generic structs and unions

A struct or union takes type parameters after its name, and its fields may
use them. The type is applied to its arguments by position, without
parentheses: `pair i32 str` is a pair of an `i32` and a `str`.

```chasm
struct pair T U  first: T  second: U

: swap-pair ( pair T U -- pair U T )  :> p  p pair.second p pair.first pair.new ;

test pair.second : 3 "x" pair.new pair.second -> "x"
test swap-pair : 3 "x" pair.new swap-pair pair.first -> "x"
test swap-pair : 1.5 7 pair.new swap-pair pair.second -> 1.5
```

The generated words are generic words: `pair.new ( T U -- pair T U )`
works out `T` and `U` from its arguments, so `3 "x" pair.new` is a
`pair i32 str`. `swap-pair` is a generic word over a generic type, and
works on every pair.

## When the type cannot be worked out

Some constructors take no argument that mentions the parameter. An empty
list could be a list of anything, so `list.nil` with nothing to fix its
type is `E_AMBIGUOUS_TYPE`. A declared effect settles it, as in `empty`
below, or a stack assertion: `list.nil ( list str )`.

```chasm
union list T
  | nil
  | cons  head: T  tail: list T

: length ( list T -- i32 )
  nil:  [ 0 ]
  cons: [ nip length 1 i32.add ]
  match ;

: empty ( -- list i32 )  list.nil ;
: three ( -- list i32 )  1 2 3 empty list.cons list.cons list.cons ;

test length : three length -> 3
test length : list.nil ( list str ) length -> 0
```

In `length` the `cons:` arm receives the head and the tail; `nip` discards
the head, and the word calls itself on the tail. It never looks at an
element, so it works for a list of any type.

The same rule covers function values. `'twice` does not say which `twice`
is meant, so something must fix it, such as an assertion naming the
function type: `4 'twice ( i32 [ i32 -- i32 i32 ] ) call` leaves 4 twice.

Next: [Collections](10-collections.md)
