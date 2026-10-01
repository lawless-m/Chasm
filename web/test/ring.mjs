// Check web/ring.js: submit entries by hand and service them.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { loadCompiler } from "../compiler.js";
import { Namespace, serviceRing } from "../ring.js";

const { layout: L } = await loadCompiler(
  readFileSync(new URL("../chasm_web.wasm", import.meta.url)),
);
const memory = new WebAssembly.Memory({
  initial: L.INITIAL_PAGES,
  maximum: L.SHARED_MAX_PAGES,
  shared: true,
});
let output = "";
const ns = new Namespace(L, { onOutput: (t) => (output += t) });
const dv = new DataView(memory.buffer);
const u8 = new Uint8Array(memory.buffer);
const enc = new TextEncoder();

function submit(op, a0, a1, a2) {
  const tail = dv.getUint32(L.SQ_TAIL, true);
  const e = L.SQ_BASE + (tail % L.RING_ENTRIES) * L.SQE_SIZE;
  dv.setInt32(e + L.SQE_OP, op, true);
  dv.setUint32(e + L.SQE_USER, tail, true);
  dv.setInt32(e + L.SQE_A0, a0, true);
  dv.setInt32(e + L.SQE_A1, a1, true);
  dv.setInt32(e + L.SQE_A2, a2, true);
  dv.setUint32(L.SQ_TAIL, tail + 1, true);
  serviceRing(memory, L, ns);
  const head = dv.getUint32(L.CQ_HEAD, true);
  const c = L.CQ_BASE + (head % L.RING_ENTRIES) * L.CQE_SIZE;
  dv.setUint32(L.CQ_HEAD, head + 1, true);
  return dv.getInt32(c + L.CQE_RESULT, true);
}

const put = (addr, s) => {
  const b = enc.encode(s);
  u8.set(b, addr);
  return b.length;
};

const plen = put(0x300000, "/dev/cons");
const h = submit(L.OP_OPEN, 0x300000, plen, L.MODE_WRITE);
assert.ok(h >= 3);
assert.equal(submit(L.OP_WRITE, h, 0x300100, put(0x300100, "hi")), 2);
assert.equal(dv.getUint32(L.SQ_HEAD, true), 2);
assert.equal(dv.getUint32(L.SQ_TAIL, true), 2);
assert.equal(dv.getUint32(L.CQ_TAIL, true), 2);
assert.equal(output, "hi");

assert.equal(submit(L.OP_OPEN, 0x300000, put(0x300000, "/nope"), L.MODE_READ), L.E_NOT_FOUND);

const t = submit(L.OP_OPEN, 0x300000, put(0x300000, "/dev/time"), L.MODE_READ);
assert.equal(submit(L.OP_READ, t, 0x300200, 8), 8);
assert.ok(dv.getBigUint64(0x300200, true) > 1_600_000_000_000_000_000n);
assert.equal(submit(L.OP_READ, t, 0x300200, 8), 0);
assert.equal(submit(L.OP_CLOSE, t, 0, 0), 0);
assert.equal(submit(L.OP_CLOSE, t, 0, 0), L.E_BAD_HANDLE);

console.log("ring.mjs ok");
