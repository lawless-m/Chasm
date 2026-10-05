# Whackford

Whackford is a typed, concatenative language in the Forth and Factor family.
Every word has a stack effect, written or inferred, and the checker verifies
each body against it before anything runs. Programs compile to WebAssembly
and run natively or in the browser.

```wack
: square ( i32 -- i32 )  dup i32.mul ;
test square : 3 square -> 9

: main ( -- )  "7 squared is " print  7 square i32.to-str println ;
```

- [Tour](tour/index.html): the language, one idea at a time
- [Reference](reference.html): every form, primitive and diagnostic
- [Words](words.html): each word with its effect
- [REPL](repl/): try it in the browser
- [GitHub](https://github.com/lawless-m/Whackford): the source
