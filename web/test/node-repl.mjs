// End-to-end check of the browser REPL under node: the compiler and driver
// on the main thread, worker-core.js in a worker thread, shared memory,
// Atomics doorbell.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { Worker } from "node:worker_threads";
import { loadCompiler } from "../compiler.js";
import { Repl } from "../driver.js";

const compiler = await loadCompiler(readFileSync(new URL("../chasm_web.wasm", import.meta.url)));
const L = compiler.layout;
const memory = new WebAssembly.Memory({ initial: L.INITIAL_PAGES, maximum: L.SHARED_MAX_PAGES, shared: true });
const worker = new Worker(new URL("./node-worker.mjs", import.meta.url));
let output = "";
const repl = new Repl({
  compiler,
  memory,
  worker: { post: (m) => worker.postMessage(m), onMessage: (h) => worker.on("message", h) },
  onOutput: (t) => (output += t),
});

const n = (v) => [{ type: "i32", value: String(v) }];
try {
  let r = await repl.step(": sq ( i32 -- i32 ) dup i32.mul ;");
  assert.ok(r.ok, JSON.stringify(r.diagnostics));
  assert.equal(r.defined.length, 1);

  r = await repl.step("3 sq");
  assert.deepEqual(r.stack, n(9));

  r = await repl.step('"hi" println');
  assert.ok(output.includes("hi\n"), output);
  assert.deepEqual(r.stack, n(9));

  r = await repl.step("1 0 i32.div_s");
  assert.ok(r.trap);
  assert.deepEqual(r.stack, n(9));

  await repl.step(": twice ( i32 -- i32 ) sq sq ;");
  await repl.step(": sq ( i32 -- i32 ) 2 i32.mul ;");
  r = await repl.step("drop 3 twice");
  assert.deepEqual(r.stack, n(12));

  r = await repl.step("test sq : 3 sq -> 6");
  assert.equal(r.tests.length, 1);
  assert.equal(r.tests[0].status, "pass");

  r = await repl.step("test sq : 3 sq -> 7");
  assert.equal(r.tests[0].status, "fail");
  assert.deepEqual(r.tests[0].actual, ["6"]);

  r = await repl.step('"boom" trap');
  assert.equal(r.trap.message, "boom");
  assert.deepEqual(r.stack, n(12));

  r = await repl.step('drop "a" "b" str.concat 2.5 7 i64');
  assert.deepEqual(r.stack, [
    { type: "str", value: '"ab"' },
    { type: "f64", value: "2.5" },
    { type: "i64", value: "7 i64" },
  ]);

  r = await repl.step(")forget sq");
  assert.equal(r.diagnostics[0].code, "E_FORGET");
  assert.deepEqual(r.diagnostics[0].dependants, ["twice"]);
  r = await repl.step(")forget twice");
  assert.deepEqual(r.forgotten, ["twice"]);
  assert.equal(r.stack.length, 3);

  console.log("node-repl.mjs ok");
  await worker.terminate();
} catch (e) {
  console.error(e);
  await worker.terminate();
  process.exit(1);
}
