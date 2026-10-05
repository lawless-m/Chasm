// Check /net/http in web/ring.js against a local node:http server.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import http from "node:http";
import { loadCompiler } from "../compiler.js";
import { Namespace, serviceRing } from "../ring.js";

const { layout: L } = await loadCompiler(readFileSync(new URL("../wack_web.wasm", import.meta.url)));

const server = http.createServer((req, res) => {
  const chunks = [];
  req.on("data", (c) => chunks.push(c));
  req.on("end", () => {
    const body = Buffer.concat(chunks).toString();
    const wack = req.headers["x-wack"] ?? "none";
    const reply = (status, text) => {
      res.writeHead(status, { "Content-Length": Buffer.byteLength(text) });
      res.end(text);
    };
    if (req.method === "GET" && req.url === "/hello") reply(200, "hi\n");
    else if (req.method === "GET" && req.url === "/echo") reply(200, wack);
    else if (req.method === "POST" && req.url === "/post") reply(200, `got:${body}:${wack}`);
    else reply(404, "");
  });
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `/net/http/127.0.0.1:${server.address().port}`;

const memory = new WebAssembly.Memory({ initial: L.INITIAL_PAGES, maximum: L.SHARED_MAX_PAGES, shared: true });
const ns = new Namespace(L, { onOutput: () => {} });
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

// Open, write what is given, read everything; a negative code is returned as is.
async function request(path, written) {
  const h = await submit(L.OP_OPEN, 0x300000, put(0x300000, path), L.MODE_READ_WRITE);
  assert.ok(h >= 0, `open ${path}: ${h}`);
  if (written) assert.equal(await submit(L.OP_WRITE, h, 0x300100, put(0x300100, written)), enc.encode(written).length);
  let out = "";
  for (;;) {
    const n = await submit(L.OP_READ, h, 0x310000, 4);
    if (n < 0) return n;
    if (n === 0) break;
    out += new TextDecoder().decode(u8.slice(0x310000, 0x310000 + n));
  }
  assert.equal(await submit(L.OP_CLOSE, h, 0, 0), 0);
  return out;
}

try {
  assert.equal(await request(`${base}/hello`, ""), "hi\n");
  assert.equal(await request(`${base}/echo`, "X-Wack: 42\n\n"), "42");
  assert.equal(await request(`${base}/post`, "X-Wack: 7\n\nhello"), "got:hello:7");
  assert.equal(await request(`${base}/post`, "X: 1\nbody"), L.E_MALFORMED);
  assert.equal(await request(`${base}/missing`, ""), L.E_NOT_FOUND);
  const h = await submit(L.OP_OPEN, 0x300000, put(0x300000, `${base}/hello`), L.MODE_READ_WRITE);
  assert.equal(await submit(L.OP_READ, h, 0x310000, 16), 3);
  assert.equal(await submit(L.OP_WRITE, h, 0x300100, put(0x300100, "late")), L.E_PERMISSION);
  assert.equal(await submit(L.OP_OPEN, 0x300000, put(0x300000, "/net/gopher/x"), L.MODE_READ), L.E_NOT_SUPPORTED);
  console.log("net.mjs ok");
} finally {
  server.close();
}
