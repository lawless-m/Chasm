// Port of `escape` in examples/mandelbrot.wack and `grid-sum` in bench/mandelbrot.wack.
use std::time::Instant;

fn escape(cx: f64, cy: f64, max: i32) -> i32 {
    let (mut x, mut y, mut k) = (0.0f64, 0.0f64, 0i32);
    while k < max && x * x + y * y <= 4.0 {
        let nx = x * x - y * y + cx;
        y = 2.0 * x * y + cy;
        x = nx;
        k += 1;
    }
    k
}

fn grid_sum(w: i32, h: i32, max: i32) -> i32 {
    let mut total = 0i32;
    for r in 0..h {
        let cy = 1.2 - r as f64 * 2.4 / h as f64;
        for c in 0..w {
            let cx = -2.0 + c as f64 * 3.0 / w as f64;
            total = total.wrapping_add(escape(cx, cy, max));
        }
    }
    total
}

fn main() {
    let t0 = Instant::now();
    let sum = grid_sum(1200, 800, 200);
    println!("{sum} {}", t0.elapsed().as_nanos());
}
