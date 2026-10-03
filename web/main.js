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
    if (d.dependants?.length) print(`    dependants: ${d.dependants.join(", ")}\n`, "err");
  }
  for (const d of r.defined) {
    const note = d.declared ? " (declared)" : d.inferred ? " (inferred)" : "";
    print(`ok: ${d.name} ${d.effect}${note}\n`);
  }
  for (const w of r.forgotten) print(`forgot: ${w}\n`);
  for (const f of r.forced) print(`forced: ${f.name} ${f.from} -> ${f.to}\n`);
  if (r.rechecked.length) print(`rechecked: ${r.rechecked.join(", ")}\n`);
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
  if (s.some((e) => e.value.startsWith(`${e.type}{`))) {
    print(`(\n${s.map((e) => `${e.type} ${e.value}\n`).join("")})\n`, "stack");
  } else {
    print(s.length ? `( ${s.map((e) => e.type).join(" ")} ) ${s.map((e) => e.value).join(" ")}\n` : "( )\n", "stack");
  }
}

// The program so far: every chunk that defined, declared, tested, forgot or
// forced something without an error, in order. Lines are not kept. A reload
// replays it into a fresh session.
const SAVED = "chasm.program";

// Every chunk entered, oldest first, for Up and Down in the input box.
const HISTORY = "chasm.history";
const HISTORY_MAX = 500;

function loadSaved(key = SAVED) {
  try {
    return JSON.parse(localStorage.getItem(key)) ?? [];
  } catch {
    return [];
  }
}

function save(chunks, key = SAVED) {
  try {
    localStorage.setItem(key, JSON.stringify(chunks));
  } catch {
    // Storage blocked or full: the session still works, it just is not kept.
  }
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
  const program = [];
  for (const text of loadSaved()) {
    const r = await repl.step(text);
    if (r.diagnostics.some((d) => d.severity === "error")) {
      print(text.replace(/^/gm, "> ") + "\n", "echo");
      render(r);
    } else {
      program.push(text);
    }
  }
  if (program.length) {
    print(`restored ${program.length} chunks from this browser; )program lists them, )clear forgets them\n`, "hint");
  }
  save(program);
  input.disabled = false;
  input.focus();

  const history = loadSaved(HISTORY);
  let at = history.length; // history.length is the line being typed
  let draft = "";
  input.addEventListener("keydown", (e) => {
    if ((e.key !== "ArrowUp" && e.key !== "ArrowDown") || e.shiftKey || e.altKey || e.ctrlKey || e.metaKey) return;
    // Only from the first line going up or the last line going down, so the
    // arrows still move the caret inside a multi-line chunk.
    const v = input.value;
    const up = e.key === "ArrowUp";
    if (up ? v.lastIndexOf("\n", input.selectionStart - 1) !== -1 : v.indexOf("\n", input.selectionEnd) !== -1) return;
    const next = at + (up ? -1 : 1);
    if (next < 0 || next > history.length) return;
    e.preventDefault();
    if (at === history.length) draft = v;
    at = next;
    input.value = at === history.length ? draft : history[at];
    const nl = input.value.indexOf("\n");
    const end = up && nl !== -1 ? nl : input.value.length;
    input.setSelectionRange(end, end);
  });

  let busy = false;
  input.addEventListener("keydown", async (e) => {
    if (e.key !== "Enter" || e.shiftKey || busy) return;
    const text = input.value;
    if (compiler.needsMore(text + "\n")) {
      prompt.textContent = ".";
      return; // let the newline go in
    }
    e.preventDefault();
    input.value = "";
    prompt.textContent = ">";
    if (!text.trim()) return;
    if (history.at(-1) !== text) {
      history.push(text);
      history.splice(0, history.length - HISTORY_MAX);
      save(history, HISTORY);
    }
    at = history.length;
    draft = "";
    print(text.replace(/^/gm, "> ") + "\n", "echo");
    if (text.trim() === ")program") {
      print(program.length ? program.join("\n") + "\n" : "(nothing saved)\n");
      return;
    }
    if (text.trim() === ")clear") {
      save([]);
      location.reload();
      return;
    }
    busy = true;
    try {
      const r = await repl.step(text);
      render(r);
      if (!r.line && !r.diagnostics.some((d) => d.severity === "error")) {
        program.push(text);
        save(program);
      }
    } finally {
      busy = false;
    }
  });
}

main().catch((e) => print(`${e}\n`, "err"));
