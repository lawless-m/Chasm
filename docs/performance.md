# Chasm: Run Speed

How fast compiled Chasm runs, against the same algorithms in Rust and
JavaScript. `python3 bench/run.py --record` runs the set and appends a dated
section here.

## Method

- Each task is a program from `examples/` plus a file in `bench/` that
  replaces its `main`, so the benchmark times the corpus's own code.
  `bench/rust/` and `bench/js/` hold line-for-line ports of the same
  algorithms: Rust built with `rustc -C opt-level=3`, JavaScript on node with
  typed arrays.
- Every program times its own work with its clock (`now`, `Instant`,
  `process.hrtime`) and prints a checksum and the nanoseconds. Start-up and
  compilation are not counted: `chasm run` compiles the module with wasmtime
  first. The checksums of the three must agree, or the run stops.
- Each figure is the median of five runs.
- Chasm runs through `chasm run`: whole-program compilation with direct
  calls, under wasmtime with Cranelift. Arrays are bounds-checked in all
  three languages.

| Task | Work | Checksum |
|---|---|---|
| sieve | primes below 10,000,000 (`examples/sieve.chasm`) | 664579 primes |
| mandelbrot | escape counts over a 1200 x 800 grid, at most 200 iterations | total of the counts |
| n-queens | solutions of 12 queens, recursive backtracking | 14200 |
| quicksort | Lomuto quicksort of 1,000,000 pseudo-random 31-bit integers | the middle element |

## Results

### 2026-10-02

commit 9f7428d; rustc 1.99.0 (b940084d7 2026-09-28); node v20.19.6; AMD Ryzen 5 5500; median of 5 runs.

| Task | Chasm ms | Rust ms | JS ms | Chasm / Rust | Chasm / JS |
|---|---:|---:|---:|---:|---:|
| sieve | 93.7 | 78.7 | 206.9 | 1.19 | 0.45 |
| mandelbrot | 115.9 | 106.6 | 111.3 | 1.09 | 1.04 |
| n-queens | 110.0 | 80.7 | 113.7 | 1.36 | 0.97 |
| quicksort | 93.3 | 55.2 | 139.8 | 1.69 | 0.67 |
