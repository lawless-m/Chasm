// Port of `primes` in examples/sieve.wack.
function primes(n) {
  const composite = new Int32Array(n + 1);
  for (let p = 2; p * p <= n; p++) {
    if (composite[p] === 0) for (let m = p * p; m <= n; m += p) composite[m] = 1;
  }
  const all = new Int32Array(n + 1);
  for (let i = 0; i < all.length; i++) all[i] = i;
  return all.filter((k) => k >= 2 && composite[k] === 0);
}

const t0 = process.hrtime.bigint();
const count = primes(10_000_000).length;
console.log(`${count} ${process.hrtime.bigint() - t0}`);
