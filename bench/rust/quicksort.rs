// Port of `quicksort` in examples/quicksort.chasm and `wide` in bench/quicksort.chasm.
use std::time::Instant;

fn partition(a: &mut [i32]) -> usize {
    let hi = a.len() - 1;
    let pivot = a[hi];
    let mut store = 0;
    for i in 0..hi {
        if a[i] < pivot {
            a.swap(i, store);
            store += 1;
        }
    }
    a.swap(store, hi);
    store
}

fn quicksort(a: &mut [i32]) {
    if a.len() > 1 {
        let p = partition(a);
        let (left, right) = a.split_at_mut(p);
        quicksort(left);
        quicksort(&mut right[1..]);
    }
}

fn wide(n: usize) -> Vec<i32> {
    let mut x: i32 = 12345;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1103515245).wrapping_add(12345) & 0x7FFF_FFFF;
            x
        })
        .collect()
}

fn main() {
    let mut a = wide(1_000_000);
    let t0 = Instant::now();
    quicksort(&mut a);
    let ns = t0.elapsed().as_nanos();
    let check = if a.windows(2).all(|w| w[0] <= w[1]) { a[500_000] } else { -1 };
    println!("{check} {ns}");
}
