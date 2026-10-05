// End-to-end check of the browser REPL under node: the compiler and driver
// on the main thread, worker-core.js in a worker thread, shared memory,
// Atomics doorbell.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { Worker } from "node:worker_threads";
import { loadCompiler } from "../compiler.js";
import { Repl } from "../driver.js";

const compiler = await loadCompiler(readFileSync(new URL("../wack_web.wasm", import.meta.url)));
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

  r = await repl.step(": tick ( -- [ i32 -- i32 ] ) 'sq ;");
  assert.ok(r.ok, JSON.stringify(r.diagnostics));
  r = await repl.step(")force : sq ( i32 -- i64 ) i64 ;\n: tick ( -- [ i32 -- i32 ] ) 'sq ;\n");
  assert.equal(r.diagnostics[0].code, "E_FORCE");
  r = await repl.step(")force : sq ( i32 -- i64 ) i64 ;\n: tick ( -- [ i32 -- i64 ] ) 'sq ;\n");
  assert.ok(r.ok, JSON.stringify(r.diagnostics));
  assert.equal(r.forced[0].to, "( i32 -- i64 )");
  r = await repl.step("drop drop drop 3 sq");
  assert.deepEqual(r.stack, [{ type: "i64", value: "3 i64" }]);

  r = await repl.step(": cube dup dup i32.mul i32.mul ;");
  assert.ok(r.ok, JSON.stringify(r.diagnostics));
  assert.equal(r.defined[0].effect, "( i32 -- i32 )");
  assert.equal(r.defined[0].inferred, true);
  r = await repl.step(": twice ( T -- T T ) dup ;");
  assert.equal(r.defined[0].effect, "( T -- T T )");
  r = await repl.step("drop 2 cube twice");
  assert.deepEqual(r.stack, [
    { type: "i32", value: "8" },
    { type: "i32", value: "8" },
  ]);
  r = await repl.step('drop drop "ab" twice str.concat');
  assert.deepEqual(r.stack, [{ type: "str", value: '"abab"' }]);
  r = await repl.step("test twice : 1.5 twice f64.add -> 3.0");
  assert.equal(r.tests[0].status, "pass");

  r = await repl.step("declare stub ( -- i32 )");
  r = await repl.step("test stub : stub -> 1");
  r = await repl.step(")test twice");
  assert.ok(r.tested && r.ok);
  assert.deepEqual(r.tests.map((t) => [t.word, t.status]), [["twice", "pass"]]);
  r = await repl.step(")test");
  assert.deepEqual(r.tests.map((t) => [t.word, t.status]), [["twice", "pass"], ["stub", "pending"]]);
  r = await repl.step(")forget stub");

  r = await repl.step(")words");
  assert.equal(
    r.listing,
    ": sq ( i32 -- i64 ) i64 ;\n: tick ( -- [ i32 -- i64 ] ) 'sq ;\n" +
      ": cube dup dup i32.mul i32.mul ;\n: twice ( T -- T T ) dup ;\n" +
      "\ntest twice : 1.5 twice f64.add -> 3.0\n",
  );

  output = "";
  r = await repl.step('drop "kept\\n" "/local/g" write-file "/local/g" "/local/h" copy "/local/h" read-file drop print "/local" ls');
  assert.ok(r.ok, JSON.stringify(r.diagnostics));
  assert.deepEqual(r.stack, [...n(0), ...n(0), ...n(0)]);
  assert.equal(output, "kept\ng\nh\n");
  r = await repl.step('drop drop drop "/local/none" "/local/x" copy');
  assert.deepEqual(r.stack, n(-1));

  r = await repl.step('drop "/local/g" 0 host.open :> h  4 bytes.new :> b  h b host.read  b  b 0 2 bytes.slice bytes.to-str');
  assert.ok(r.ok, JSON.stringify(r.diagnostics));
  assert.deepEqual(r.stack, [...n(4), { type: "bytes", value: "<4 bytes>" }, { type: "str", value: '"ke"' }]);
  r = await repl.step("drop 9 bytes.at");
  assert.equal(r.trap.message, "bytes.at: offset out of range");

  r = await repl.step("drop drop 8 mem.alloc");
  assert.equal(r.diagnostics[0].code, "E_RAW");
  r = await repl.step("raw : cell ( -- i32 ) 8 mem.alloc i32.load ;");
  assert.ok(r.ok, JSON.stringify(r.diagnostics));
  r = await repl.step("drop drop drop cell");
  assert.deepEqual(r.stack, n(0));

  console.log("node-repl.mjs ok");
  await worker.terminate();
} catch (e) {
  console.error(e);
  await worker.terminate();
  process.exit(1);
}
