# Chasm: Example Corpus and Benchmark

Status: draft v0.3. The programs live in `examples/`.

## Purpose

One set of programs does three jobs:

1. **Acceptance suite** for the compiler: every file in `examples/` must check and pass its tests at every milestone. `crates/cli/tests/examples.rs` enforces this, running `chasm check` and `chasm test` on each file separately (passing several files to one command compiles them as one program).
2. **Example corpus**: the main way a model (or a person) learns idiomatic Chasm, since the language is in no training data.
3. **Benchmark** for how well Claude Code writes Chasm, tracked over time.

Tasks are taken from Rosetta Code **task descriptions** only. Every solution is written fresh for Chasm.

## Conventions

- One file per task: `examples/<task>.chasm`, lower-case, hyphens.
- Tests inline, using `test` lines, so the file is self-checking. Prefer tests over printed output where the task allows it; where output is the task, test the words that build the output and keep `main` thin.
- Each file begins with a `#` comment giving the task, the Rosetta Code task name, and which features it exercises.
- No feature beyond what the compiler has. If a task cannot be done cleanly without a missing feature, say so in the file header and record it under "Walls hit".
- `test` compares the stack exactly and does not look at console output. Test words that return values; keep `main` as the thin printing wrapper.
- Idiomatic over clever. These are the programs the model will imitate.

The files already in `examples/` (`hello`, `basics`, `strings`, `arrays`, `contract`, `files`) are the core teaching set and stay alongside the tasks below.

## Task list

Everything below can be written with the compiler as it stands: numerics, control flow, locals, strings, arrays, inlined combinators, functions as values (`'word`, quotation values, `call`), and the namespace. Tasks marked **wall** are expected to run into a missing feature; that is their purpose.

### Easy

| Task | Exercises |
|---|---|
| FizzBuzz | `i32.rem_u`, `if`, `times`, `print` |
| Factorial | recursion with declared effect, `i64` |
| Greatest common divisor | `while`, `swap` |
| Fibonacci | locals, mutable locals, `times` |
| Leap year | boolean `i32` logic |
| Sum of digits | `i32.div_u`, `until` |
| 99 bottles of beer | string building, `str.concat`, number formatting (library) |
| Temperature conversion | `f64`, `f64.convert_i32_s`, formatting |
| Collatz sequence | `while`, `leave`, `i64` |

### Medium

| Task | Exercises |
|---|---|
| Sieve of Eratosthenes | `array.new`, `array.at!`, nested loops |
| Insertion sort | in-place array mutation, `each` with index via `times` |
| Binary search | `while` with `leave`, bounds reasoning |
| Caesar cipher | `str.cp-at`, building a new string, ASCII arithmetic |
| Reverse a string | `str.cp-at` walked forward, output assembled backward; the UTF-8 test |
| Luhn test | string to digits, `fold` |
| 100 doors | `array i32` of flags, `filter` |
| Mandelbrot (ASCII) | `f64` arithmetic, nested loops, `print` per row |
| Towers of Hanoi | recursion, `str.concat` for the move log |

### Hard

| Task | Exercises |
|---|---|
| Quicksort | recursion, `array.slice`, in-place swaps |
| Conway's Game of Life | 2-D grid on a flat array, index arithmetic, generations loop |
| N-queens | recursion, backtracking, array as a stack |
| Brainfuck interpreter | tape as `array i32`, nested loops, bracket matching, output built as a `str` |
| RPN calculator | a stack language implementing a stack calculator; string tokenising, `array f64` as the stack, a hand-written power word |
| Word frequency (wall) | tokenising, parallel arrays as a poor man's map; expected to want a map type |
| Tokenizer (wall) | `str.cp-at`, state machine, token arrays; expected to want structs |
| Apply a callback to an array | `'word`, quotation values, `call`, quotation types in effects |
| Sort with a custom comparator | insertion sort taking `[ i32 i32 -- i32 ]` |
| Function composition (wall) | quotation values; expected to want closures |

## Task statements

Written in our own words so the doc stands alone. Rosetta Code pages are `https://rosettacode.org/wiki/<Task_name>` with the task name as given; use them for discussion and edge cases, not as the spec.

### Easy

**FizzBuzz** (`FizzBuzz`). Print 1 to 100, replacing multiples of 3 with `Fizz`, of 5 with `Buzz`, of both with `FizzBuzz`. Test the word that produces a line for `n`.

**Factorial** (`Factorial`). `n!` for `n` in 0 to 20 as `i64`. Recursive and iterative versions both welcome; test both.

**Greatest common divisor** (`Greatest_common_divisor`). Euclid's algorithm on two non-negative `i32`s; `gcd(0, n) = n`.

**Fibonacci** (`Fibonacci_sequence`). The n-th Fibonacci number, `F(0) = 0`, `F(1) = 1`, iteratively, as `i64` for `n` up to 90.

**Leap year** (`Leap_year`). Gregorian rule: divisible by 4, except centuries, except multiples of 400.

**Sum of digits** (`Sum_digits_of_an_integer`). Sum the decimal digits of a non-negative `i32`.

**99 bottles of beer** (`99_bottles_of_beer`). Print the song from 99 down to 0, with correct singular "bottle" at 1 and "no more bottles" at 0. Test the word that builds one verse.

**Temperature conversion** (`Temperature_conversion`). Given Kelvin as `f64`, produce Celsius, Fahrenheit and Rankine. `test` compares `f64` results exactly, so pick inputs where the arithmetic gives exactly the expected literal, or test a word that rounds to hundredths and returns an `i32`.

**Collatz sequence** (`Hailstone_sequence`). Length of the hailstone sequence from `n` (counting `n` and the final 1) and the number under 100,000 with the longest sequence. Use `i64`.

### Medium

**Sieve of Eratosthenes** (`Sieve_of_Eratosthenes`). All primes up to `n` as `array i32`, using a flag array and striking out multiples.

**Insertion sort** (`Sorting_algorithms/Insertion_sort`). Sort an `array i32` in place, ascending.

**Binary search** (`Binary_search`). Index of a value in a sorted `array i32`, or -1 if absent. Iterative.

**Caesar cipher** (`Caesar_cipher`). Shift ASCII letters by `k`, preserving case and leaving other bytes alone; decode is shift by `26 - k`.

**Reverse a string** (`Reverse_a_string`). Reverse by codepoint, not by byte: `"héllo"` reverses to `"olléh"` and remains valid UTF-8. Test with a multi-byte input.

**Luhn test** (`Luhn_test_of_credit_card_numbers`). Validate a digit string by the Luhn checksum. Test with the standard four examples (two valid, two invalid).

**100 doors** (`100_doors`). 100 closed doors; pass `i` toggles every `i`-th door, for `i` in 1 to 100. Report which are open (the squares).

**Mandelbrot (ASCII)** (`Mandelbrot_set`). Render the set as text, say 78 columns by 40 rows over the usual region, with a character per cell depending on escape iteration count. Test the escape-count word on a few known points.

**Towers of Hanoi** (`Towers_of_Hanoi`). Produce the move list for `n` discs. Test the move count is `2^n - 1` and the first few moves for `n = 3`.

### Hard

**Quicksort** (`Sorting_algorithms/Quicksort`). In-place recursive quicksort on `array i32` using `array.slice` for sub-ranges.

**Conway's Game of Life** (`Conway's_Game_of_Life`). A fixed grid stored as a flat `array i32`, dead edges, standard rules. Test that a blinker oscillates with period 2 and a block is stable.

**N-queens** (`N-queens_problem`). Count solutions for `n` from 1 to 8 (1, 0, 0, 2, 10, 4, 40, 92) by backtracking.

**Brainfuck interpreter** (`Execute_Brain****`). The eight commands, a 30,000-cell tape of `i32` wrapping at 256, nested bracket matching. The interpreter word takes the program and its input as `str` and returns the output as a `str`, so a hello-world program can be tested; `main` runs a program against `/dev/cons`.

**RPN calculator** (`Parsing/RPN_calculator_algorithm`). Evaluate a space-separated RPN string over `f64` with `+ - * / ^`; test `3 4 2 * 1 5 - 2 3 ^ ^ / +` gives 3.0001220703125 (exact in binary, so an exact test holds). There is no `f64.pow`: `^` is a word written for whole-number exponents.

**Word frequency** (`Word_frequency`). Count words in a text, lower-case, split on non-letters, report the top ten. Expected to strain without a map type; record how it was done.

**Tokenizer** (no Rosetta task; `Parsing/Lexical_analyzer` is close). Tokenise a small expression language: integers, identifiers, `+ - * / ( )`, with position information. Expected to want structs.

**Apply a callback to an array** (`Apply_a_callback_to_an_array`). Square each element via a word passed as a value with `'square`, not an inlined quotation.

**Sort with a custom comparator** (`Sort_using_a_custom_comparator`). Insertion sort taking a `[ i32 i32 -- i32 ]` comparator; test ascending and descending.

**Function composition** (`Function_composition`, wall). A word `compose` that, given two quotation values, returns one that applies both. In v1 it cannot be written: quotation values take no inputs, capture nothing, and no function can be made at run time. Write `apply-both ( i32 [ i32 -- i32 ] [ i32 -- i32 ] -- i32 )` instead and record the wall.

## Benchmark protocol

Run at the end of each milestone and whenever the examples directory grows substantially.

1. Give Claude Code **only** the task statement, `docs/reference.md` (how to write Chasm), `LANGUAGE.md`, and the `examples/` directory as it stood before this task was added. Not the existing solution.
2. It writes the file, including tests, and loops on `chasm check` and `chasm test` until they pass or it gives up.
3. Record per task: passed or not, number of checker rounds, number of test rounds, and which diagnostic codes it hit (`chasm check --json` gives the codes).
4. Keep the results in `docs/benchmark.md` as a table with one row per task per run, dated.

What to watch:
- **Checker rounds falling** as the corpus grows: the examples are teaching.
- **Which diagnostic codes recur**: a code that keeps appearing is either a confusing part of the language or a poor error message. Fix the message first, then consider the language.
- **Tasks that never pass**: a missing feature or a spec gap. Record under "Walls hit".

## Walls hit

Append as they happen. Each entry: task, what was missing, workaround used, and whether it changes `FUTURE.md` priorities.

| Task | Missing | Workaround | Note |
|---|---|---|---|
| Temperature conversion | `f64.to-str` (no float formatting in the library) | `hundredths`: scale by 100, `f64.nearest`, format the `i64` | Small; a candidate for the prelude rather than `FUTURE.md` |

## Growth

When a task is solved, its file becomes part of the corpus for the next benchmark run. Over time, prefer adding tasks that exercise a feature the corpus is thin on rather than more of the same. Keep the easy tier small; the model learns more from a few clean examples than from many repetitive ones.
