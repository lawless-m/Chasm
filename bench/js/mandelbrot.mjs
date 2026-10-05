// Port of `escape` in examples/mandelbrot.wack and `grid-sum` in bench/mandelbrot.wack.
function escape(cx, cy, max) {
  let x = 0, y = 0, k = 0;
  while (k < max && x * x + y * y <= 4.0) {
    const nx = x * x - y * y + cx;
    y = 2.0 * x * y + cy;
    x = nx;
    k++;
  }
  return k;
}

function gridSum(w, h, max) {
  let total = 0;
  for (let r = 0; r < h; r++) {
    const cy = 1.2 - (r * 2.4) / h;
    for (let c = 0; c < w; c++) {
      const cx = -2.0 + (c * 3.0) / w;
      total = (total + escape(cx, cy, max)) | 0;
    }
  }
  return total;
}

const t0 = process.hrtime.bigint();
const sum = gridSum(1200, 800, 200);
console.log(`${sum} ${process.hrtime.bigint() - t0}`);
