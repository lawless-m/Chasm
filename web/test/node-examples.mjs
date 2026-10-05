// The process examples under node, each in a fresh REPL with its worker in a
// worker thread. Needs node 22 or later (WasmGC); CI runs it.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { Worker } from "node:worker_threads";
import { loadCompiler } from "../compiler.js";
import { Repl } from "../driver.js";
import { examplesScenario } from "./examples-scenario.mjs";

const bytes = readFileSync(new URL("../wack_web.wasm", import.meta.url));
const workers = [];
const makeRepl = async () => {
  const compiler = await loadCompiler(bytes);
  const L = compiler.layout;
  const memory = new WebAssembly.Memory({ initial: L.INITIAL_PAGES, maximum: L.SHARED_MAX_PAGES, shared: true });
  const worker = new Worker(new URL("./node-worker.mjs", import.meta.url));
  workers.push(worker);
  let text = "";
  const repl = new Repl({
    compiler,
    memory,
    worker: { post: (m) => worker.postMessage(m), onMessage: (h) => worker.on("message", h) },
    onOutput: (t) => {
      text += t;
    },
  });
  return { repl, output: () => text };
};
const fetchText = async (path) => readFileSync(new URL("../.." + path, import.meta.url), "utf8");
try {
  await examplesScenario(makeRepl, assert, fetchText);
  console.log("node-examples.mjs ok");
  await Promise.all(workers.map((w) => w.terminate()));
} catch (e) {
  console.error(e);
  await Promise.all(workers.map((w) => w.terminate()));
  process.exit(1);
}
