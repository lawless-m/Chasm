// Check web/ring.js: submit entries by hand and service them.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { loadCompiler } from "../compiler.js";
import { Namespace, serviceRing } from "../ring.js";

const { layout: L } = await loadCompiler(
  readFileSync(new URL("../wack_web.wasm", import.meta.url)),
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

async function submit(op, a0, a1, a2) {
  const tail = dv.getUint32(L.SQ_TAIL, true);
  const e = L.SQ_BASE + (tail % L.RING_ENTRIES) * L.SQE_SIZE;
  dv.setInt32(e + L.SQE_OP, op, true);
  dv.setUint32(e + L.SQE_USER, tail, true);
  dv.setInt32(e + L.SQE_A0, a0, true);
  dv.setInt32(e + L.SQE_A1, a1, true);
  dv.setInt32(e + L.SQE_A2, a2, true);
  dv.setUint32(L.SQ_TAIL, tail + 1, true);
  await serviceRing(memory, L, ns);
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
const h = await submit(L.OP_OPEN, 0x300000, plen, L.MODE_WRITE);
assert.ok(h >= 3);
assert.equal(await submit(L.OP_WRITE, h, 0x300100, put(0x300100, "hi")), 2);
assert.equal(dv.getUint32(L.SQ_HEAD, true), 2);
assert.equal(dv.getUint32(L.SQ_TAIL, true), 2);
assert.equal(dv.getUint32(L.CQ_TAIL, true), 2);
assert.equal(output, "hi");

assert.equal(await submit(L.OP_OPEN, 0x300000, put(0x300000, "/nope"), L.MODE_READ), L.E_NOT_FOUND);

const t = await submit(L.OP_OPEN, 0x300000, put(0x300000, "/dev/time"), L.MODE_READ);
assert.equal(await submit(L.OP_READ, t, 0x300200, 8), 8);
assert.ok(dv.getBigUint64(0x300200, true) > 1_600_000_000_000_000_000n);
assert.equal(await submit(L.OP_READ, t, 0x300200, 8), 0);
assert.equal(await submit(L.OP_CLOSE, t, 0, 0), 0);
assert.equal(await submit(L.OP_CLOSE, t, 0, 0), L.E_BAD_HANDLE);

// `/local`: blobs in the namespace's Storage (in memory here).
const open = async (path, mode) => submit(L.OP_OPEN, 0x300000, put(0x300000, path), mode);
const read = async (h) => {
  const n = await submit(L.OP_READ, h, 0x300200, 64);
  return n < 0 ? n : new TextDecoder().decode(u8.slice(0x300200, 0x300200 + n));
};
assert.equal(await open("/local/a", L.MODE_READ), L.E_NOT_FOUND);
assert.equal(await open("/local/x/y", L.MODE_WRITE), L.E_NOT_FOUND);
const w = await open("/local/a", L.MODE_WRITE);
assert.equal(await read(w), L.E_PERMISSION);
assert.equal(await submit(L.OP_WRITE, w, 0x300100, put(0x300100, "hel")), 3);
assert.equal(await submit(L.OP_WRITE, w, 0x300100, put(0x300100, "lo")), 2);
await submit(L.OP_CLOSE, w, 0, 0);
const ap = await open("/local/a", L.MODE_APPEND);
assert.equal(await submit(L.OP_WRITE, ap, 0x300100, put(0x300100, "!")), 1);
const r = await open("/local/a", L.MODE_READ);
assert.equal(await submit(L.OP_WRITE, r, 0x300100, 1), L.E_PERMISSION);
assert.equal(await read(r), "hello!");
assert.equal(await read(r), "");
assert.equal(ns.storage.getItem("wack/local/a"), "hello!");
assert.equal(await open("/local/b", L.MODE_WRITE) >= 3, true);
assert.equal(await open("/local", L.MODE_WRITE), L.E_PERMISSION);
const d = await open("/local", L.MODE_READ);
const n = await submit(L.OP_READ, d, 0x300200, 64);
assert.equal(n, 2 * (13 + 1));
assert.equal(dv.getUint32(0x300200, true), 1);
assert.equal(u8[0x300204], "a".charCodeAt(0));
assert.equal(dv.getBigUint64(0x300205, true), 6n);
assert.equal(u8[0x300200 + 14 + 4], "b".charCodeAt(0));
assert.equal(await submit(L.OP_READ, d, 0x300200, 64), 0);

console.log("ring.mjs ok");
