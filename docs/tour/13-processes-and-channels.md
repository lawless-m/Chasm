# Processes and channels

A program can run several processes at once, passing values between them
over channels. They take turns: one runs at a time, and they switch only
when one waits on a channel or finishes. These programs run everywhere:
`wack run`, `wack test`, `wack repl` and the browser REPL (the Try-it
button).

## Spawn and channels

`spawn` takes a function value and starts it as a process. A closure
reaches only what it captured, so the channel it captures is its whole
connection to the rest of the program. `chan.send` waits until a receiver
takes the value; `chan.recv` waits for one and gives an `option`, `none`
once the channel is closed and every value has been taken.

```wack
: one-value ( -- i32 )
  chan.make ( chan i32 ) :> c
  [ c 42 chan.send  c chan.close ] spawn
  c chan.recv none: [ 0 ] some: [ ] match ;
test one-value : one-value -> 42
```

## Closing, counted

A reader learns that a channel is finished from the channel itself, never
from a special value. Every sender closes once: `chan.make` makes a channel
with one sender, and `chan.sender` adds one for each extra process that
will send. When the last sender closes, `chan.recv` gives `none`. Two
producers feeding one reader:

```wack
: produce ( chan i32 i32 -- )
  :> n :> c
  n [ 1 i32.add c swap chan.send ] times
  c chan.close ;

: sum-of ( i32 -- i32 )
  :> n
  chan.make ( chan i32 ) :> c
  c chan.sender
  [ c n produce ] spawn
  [ c n produce ] spawn
  0 :> total!
  [ 1 ]
  [
    c chan.recv
    none: [ leave ]
    some: [ total i32.add total! ]
    match
  ]
  while
  total ;
test sum-of : 3 sum-of -> 12
```

Forget a `chan.close` and the reader waits for a value that never comes.
When no process can run, the program says so rather than hanging:
`all processes blocked: [line] waits to receive on chan 1`, naming each
waiting process and its channels.

## alt

`alt` waits on several channels at once and runs the arm of the first one
with a value. Each arm is a channel, the label `recv:` and a block, which
receives an `option`:

```wack
: first-ready ( -- i32 )
  chan.make ( chan i32 ) :> a
  chan.make ( chan i32 ) :> b
  [ b 7 chan.send ] spawn
  a recv: [ none: [ -1 ] some: [ ] match ]
  b recv: [ none: [ -2 ] some: [ ] match ]
  alt ;
test first-ready : first-ready -> 7
```

## Timeouts

`time.sleep` waits a number of milliseconds while other processes run.
`time.after` takes a value and a delay and gives a channel that receives
that value once the delay has passed, so an `alt` arm on it is a timeout:

```wack
# The value from c, or -1 when ms milliseconds pass first.
: recv-within ( chan i32 i32 -- i32 )
  :> ms :> c
  -1 ms time.after :> timer
  c recv: [ none: [ -1 ] some: [ ] match ]
  timer recv: [ none: [ -1 ] some: [ ] match ]
  alt ;

: slow-producer ( -- i32 )
  chan.make ( chan i32 ) :> c
  [ 50 time.sleep  c 7 chan.send ] spawn
  c 10 recv-within ;
test slow-producer : slow-producer -> -1
```

Here the timer wins, so the slow producer stays parked on its send until
the program ends. When the other arm wins, the timer's process waits the
same way. In the REPL, `/prog` lists whichever is left.

## In the REPL

A process that is waiting stays from one step to the next, so a later line
can wake it. A line that could only wait for ever traps, and the stack is
left as it was:

```wack-repl
> : schan ( -- chan str ) chan.make ;
ok: schan ( -- chan str )
( )
> schan :> c  c  [ c chan.recv none: [ "closed" println ] some: [ println ] match ] spawn
(
chan str chan{id: 1, q: vec{chunks: <32 elements>, count: 0}, head: 0}
)
> dup "hello" chan.send
hello
(
chan str chan{id: 1, q: vec{chunks: <32 elements>, count: 0}, head: 0}
)
> drop schan chan.recv
trap in `[line]`: all processes blocked: [line] waits to receive on chan 2
(
chan str chan{id: 1, q: vec{chunks: <32 elements>, count: 0}, head: 0}
)
```

`/prog` lists the live processes, and writing `kill` to `/prog/<pid>/ctl`
ends one.

## Where next

- The [reference](../reference.md): every form, primitive and diagnostic.
- The Words page of this site: each word with its effect.
- The [examples](https://github.com/lawless-m/Whackford/tree/main/examples) on GitHub: complete programs, each with tests.
