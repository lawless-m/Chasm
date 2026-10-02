// Port of `queens` in examples/n-queens.chasm.
function safe(q, r, c) {
  for (let i = 0; i < r; i++) {
    const qc = q[i];
    if (qc === c || qc - c === r - i || c - qc === r - i) return false;
  }
  return true;
}

function solve(q, r, n) {
  if (r === n) return 1;
  let count = 0;
  for (let c = 0; c < n; c++) {
    if (safe(q, r, c)) {
      q[r] = c;
      count += solve(q, r + 1, n);
    }
  }
  return count;
}

const t0 = process.hrtime.bigint();
const count = solve(new Int32Array(12), 0, 12);
console.log(`${count} ${process.hrtime.bigint() - t0}`);
