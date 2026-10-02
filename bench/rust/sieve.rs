// Port of `primes` in examples/sieve.chasm.
use std::time::Instant;

fn primes(n: usize) -> Vec<i32> {
    let mut composite = vec![0i32; n + 1];
    let mut p = 2usize;
    while p * p <= n {
        if composite[p] == 0 {
            let mut m = p * p;
            while m <= n {
                composite[m] = 1;
                m += p;
            }
        }
        p += 1;
    }
    let mut all = vec![0i32; n + 1];
    for (i, v) in all.iter_mut().enumerate() {
        *v = i as i32;
    }
    all.into_iter()
        .filter(|&k| k >= 2 && composite[k as usize] == 0)
        .collect()
}

fn main() {
    let t0 = Instant::now();
    let count = primes(10_000_000).len();
    println!("{count} {}", t0.elapsed().as_nanos());
}
