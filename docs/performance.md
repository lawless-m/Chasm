# Whackford: Run Speed

How fast compiled Whackford runs, against the same algorithms in Rust and
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
  compilation are not counted: `wack run` compiles the module with wasmtime
  first. The checksums of the three must agree, or the run stops.
- Each figure is the median of five runs.
- Whackford runs through `wack run`: whole-program compilation with direct
  calls, under wasmtime with Cranelift. Arrays are bounds-checked in all
  three languages.

| Task | Work | Checksum |
|---|---|---|
| sieve | primes below 10,000,000 (`examples/sieve.wack`) | 664579 primes |
| mandelbrot | escape counts over a 1200 x 800 grid, at most 200 iterations | total of the counts |
| n-queens | solutions of 12 queens, recursive backtracking | 14200 |
| quicksort | Lomuto quicksort of 1,000,000 pseudo-random 31-bit integers | the middle element |

## Results

### 2026-10-02

commit 9f7428d; rustc 1.99.0 (b940084d7 2026-09-28); node v20.19.6; AMD Ryzen 5 5500; median of 5 runs.

| Task | Whackford ms | Rust ms | JS ms | Whackford / Rust | Whackford / JS |
|---|---:|---:|---:|---:|---:|
| sieve | 93.7 | 78.7 | 206.9 | 1.19 | 0.45 |
| mandelbrot | 115.9 | 106.6 | 111.3 | 1.09 | 1.04 |
| n-queens | 110.0 | 80.7 | 113.7 | 1.36 | 0.97 |
| quicksort | 93.3 | 55.2 | 139.8 | 1.69 | 0.67 |

### 2026-10-02

commit 7e201b8; rustc 1.99.0 (b940084d7 2026-09-28); node v20.19.6; AMD Ryzen 5 5500; median of 5 runs.

| Task | Whackford ms | Rust ms | JS ms | Whackford / Rust | Whackford / JS |
|---|---:|---:|---:|---:|---:|
| sieve | 95.8 | 79.7 | 208.9 | 1.20 | 0.46 |
| mandelbrot | 116.5 | 108.2 | 113.0 | 1.08 | 1.03 |
| n-queens | 109.4 | 78.9 | 114.7 | 1.39 | 0.95 |
| quicksort | 93.1 | 55.8 | 141.8 | 1.67 | 0.66 |

After M5: dead words are left out of the module and `run` does not use
`wasm-opt`, so run speed is unchanged.

## Binaryen under wasmtime

`wasm-opt` 133 at each level, applied to the module `wack run` executes
(median of five, ms, 2026-10-02):

| `wasm-opt` | sieve | mandelbrot | n-queens | quicksort |
|---|---:|---:|---:|---:|
| none | 92.9 | 113.9 | 110.5 | 92.7 |
| `-O1` | 103.5 | 116.0 | 109.7 | 93.9 |
| `-O2` | 112.3 | 125.6 | 106.5 | 89.0 |
| `-O3` | 108.9 | 126.8 | 108.2 | 82.8 |
| `-O4` | 108.5 | 122.3 | 117.6 | 81.5 |
| `-Os` | 108.5 | 124.7 | 107.7 | 90.0 |
| `--inlining-optimizing` | 94.4 | 114.5 | 110.1 | 93.7 |

Cranelift already optimises the module at load time, so Binaryen helps only
quicksort and costs sieve and mandelbrot. `build` still uses `-O3` for size;
`run` uses it only with `--opt`.

## Ring servicing

`bench/ring.wack` makes 200,000 one-byte `host.read`s on one handle,
200,000 one-byte `host.write`s, and 200,000 calls of `now` (each an open, a
read and a close of `/dev/time`: three trips through the ring). Every trip
writes a submission entry, rings the doorbell import, dispatches in
`service_ring` and takes the completion. `bench/rust/ring.rs` makes the same
system calls directly: unbuffered one-byte `read` and `write`, and
`SystemTime::now`. The difference is the ring's own cost.

2026-10-02, commit 00daa53, AMD Ryzen 5 5500, median of five runs:

| Operation | Whackford ns | Rust ns | Ring cost ns |
|---|---:|---:|---:|
| 1-byte read | 586 | 415 | 171 |
| 1-byte write | 886 | 682 | 204 |
| `now` (3 trips) | 1576 | 1208 | 368 |

A trip through the ring costs about 120 to 200 ns, under half the system call
it carries. An io_uring backend is not built (`ARCHITECTURE.md` section 18):
a Whackford host word submits one entry and waits for it, so there is nothing to
batch.


### 2026-10-08

commit a7b0c33; rustc 1.99.0 (b940084d7 2026-09-28); node v22.23.3; AMD Ryzen 5 5500; median of 5 runs.

| Task | Whackford ms | Rust ms | JS ms | Whackford / Rust | Whackford / JS |
|---|---:|---:|---:|---:|---:|
| sieve | 96.7 | 86.2 | 189.5 | 1.12 | 0.51 |
| mandelbrot | 115.9 | 106.7 | 111.5 | 1.09 | 1.04 |
| n-queens | 105.8 | 80.0 | 104.4 | 1.32 | 1.01 |
| quicksort | 69.6 | 56.3 | 121.4 | 1.24 | 0.57 |

The record above was taken with the native engine's `compiler_inlining`
on (`ARCHITECTURE.md` section 28 item 3), in the working tree after commit
a7b0c33. The same run without it gave sieve 99.6 ms, mandelbrot 115.4 ms,
n-queens 109.2 ms and quicksort 92.3 ms. The JPEG decoder's 12-megapixel
baseline decode goes from 0.95 s to 0.50 s and its arithmetic decode from
1.54 s to 0.75 s, every output byte-identical.
