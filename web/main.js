// The browser REPL page: wires the compiler, the shared memory and the
// worker together, and renders each step as the CLI's text mode does.
import { loadCompiler } from "./compiler.js";
import { Repl } from "./driver.js";

const out = document.getElementById("out");
const input = document.getElementById("in");
const prompt = document.getElementById("prompt");

function print(text, cls) {
  const span = document.createElement("span");
  if (cls) span.className = cls;
  span.textContent = text;
  out.append(span);
  out.scrollTop = out.scrollHeight;
}

function render(r) {
  for (const d of r.diagnostics) {
    const l = d.location;
    print(`${l.file}:${l.line}:${l.column}: ${d.severity}[${d.code}]: ${d.message}\n`, "err");
    if (d.expected && d.actual) {
      print(`    expected: ( ${d.expected.join(" ")} )\n    actual:   ( ${d.actual.join(" ")} )\n`, "err");
    }
  }
  for (const d of r.defined) print(`ok: ${d.name} ${d.effect}${d.declared ? " (declared)" : ""}\n`);
  for (const t of r.tests) {
    if (t.status === "pass") {
      print(`PASS     ${t.word}\n`);
      continue;
    }
    print(`FAIL     ${t.word}\n    expected: ${t.expected.join(" ")}\n`, "err");
    if (t.actual) print(`    actual:   ${t.actual.join(" ")}\n`, "err");
    if (t.trap) print(`    trap in \`${t.trap.word}\`: ${t.trap.message}\n`, "err");
  }
  if (r.trap) print(r.trap.word ? `trap in \`${r.trap.word}\`: ${r.trap.message}\n` : `trap: ${r.trap.message}\n`, "err");
  const s = r.stack;
  print(s.length ? `( ${s.map((e) => e.type).join(" ")} ) ${s.map((e) => e.value).join(" ")}\n` : "( )\n", "stack");
}

async function main() {
  if (!crossOriginIsolated) {
    print(
      "This page needs SharedArrayBuffer, so it must be served with the headers\n" +
        "  Cross-Origin-Opener-Policy: same-origin\n" +
        "  Cross-Origin-Embedder-Policy: require-corp\n" +
        "Serve it with `python3 web/serve.py`.\n",
      "err",
    );
    return;
  }
  const compiler = await loadCompiler(await (await fetch("chasm_web.wasm")).arrayBuffer());
  const L = compiler.layout;
  const memory = new WebAssembly.Memory({ initial: L.INITIAL_PAGES, maximum: L.SHARED_MAX_PAGES, shared: true });
  const worker = new Worker("worker.js", { type: "module" });
  const repl = new Repl({
    compiler,
    memory,
    worker: {
      post: (m) => worker.postMessage(m),
      onMessage: (h) => worker.addEventListener("message", (e) => h(e.data)),
    },
    onOutput: (t) => print(t, "program"),
  });
  await repl.ready;
  print("Chasm REPL. Type a definition or a line; Enter runs it.\n", "hint");
  input.disabled = false;
  input.focus();

  let busy = false;
  input.addEventListener("keydown", async (e) => {
    if (e.key !== "Enter" || e.shiftKey || busy) return;
    const text = input.value;
    if (compiler.needsMore(text)) {
      prompt.textContent = ".";
      return; // let the newline go in
    }
    e.preventDefault();
    input.value = "";
    prompt.textContent = ">";
    if (!text.trim()) return;
    print(text.replace(/^/gm, "> ") + "\n", "echo");
    busy = true;
    try {
      render(await repl.step(text));
    } finally {
      busy = false;
    }
  });
}

main().catch((e) => print(`${e}\n`, "err"));
