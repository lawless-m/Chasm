// Port of `quicksort` in examples/quicksort.wack and `wide` in bench/quicksort.wack.
function swapAt(a, i, j) {
  const t = a[i];
  a[i] = a[j];
  a[j] = t;
}

function partition(a) {
  const hi = a.length - 1;
  const pivot = a[hi];
  let store = 0;
  for (let i = 0; i < hi; i++) {
    if (a[i] < pivot) {
      swapAt(a, i, store);
      store++;
    }
  }
  swapAt(a, store, hi);
  return store;
}

function quicksort(a) {
  if (a.length > 1) {
    const p = partition(a);
    quicksort(a.subarray(0, p));
    quicksort(a.subarray(p + 1));
  }
}

function wide(n) {
  const a = new Int32Array(n);
  let x = 12345;
  for (let i = 0; i < n; i++) {
    x = (Math.imul(x, 1103515245) + 12345) & 0x7fffffff;
    a[i] = x;
  }
  return a;
}

const a = wide(1_000_000);
const t0 = process.hrtime.bigint();
quicksort(a);
const ns = process.hrtime.bigint() - t0;
let sorted = true;
for (let i = 0; i + 1 < a.length; i++) if (a[i] > a[i + 1]) sorted = false;
console.log(`${sorted ? a[500000] : -1} ${ns}`);
