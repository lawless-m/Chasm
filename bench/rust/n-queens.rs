// Port of `queens` in examples/n-queens.chasm.
use std::time::Instant;

fn safe(q: &[i32], r: i32, c: i32) -> bool {
    for i in 0..r {
        let qc = q[i as usize];
        if qc == c || qc - c == r - i || c - qc == r - i {
            return false;
        }
    }
    true
}

fn solve(q: &mut [i32], r: i32, n: i32) -> i32 {
    if r == n {
        return 1;
    }
    let mut count = 0;
    for c in 0..n {
        if safe(q, r, c) {
            q[r as usize] = c;
            count += solve(q, r + 1, n);
        }
    }
    count
}

fn main() {
    let t0 = Instant::now();
    let mut q = vec![0i32; 12];
    let count = solve(&mut q, 0, 12);
    println!("{count} {}", t0.elapsed().as_nanos());
}
