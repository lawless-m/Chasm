// The process scenario under node, with the worker in a worker thread.
// Needs node 22 or later (WasmGC); CI runs it.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { Worker } from "node:worker_threads";
import { loadCompiler } from "../compiler.js";
import { Repl } from "../driver.js";
import { procScenario } from "./procs-scenario.mjs";

const compiler = await loadCompiler(readFileSync(new URL("../wack_web.wasm", import.meta.url)));
const L = compiler.layout;
const memory = new WebAssembly.Memory({ initial: L.INITIAL_PAGES, maximum: L.SHARED_MAX_PAGES, shared: true });
const worker = new Worker(new URL("./node-worker.mjs", import.meta.url));
let output = "";
const repl = new Repl({
  compiler,
  memory,
  worker: { post: (m) => worker.postMessage(m), onMessage: (h) => worker.on("message", h) },
  onOutput: (text) => {
    output += text;
  },
});
try {
  await procScenario(repl, assert, () => output);
  console.log("node-procs.mjs ok");
  await worker.terminate();
} catch (e) {
  console.error(e);
  await worker.terminate();
  process.exit(1);
}
