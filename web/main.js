// The browser REPL page: wires the compiler, the shared memory and the
// worker together, and renders each step as the CLI's text mode does.
import { loadCompiler } from "./compiler.js";
import { chunks, Repl } from "./driver.js";

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
  if (r.listing != null) print(r.listing || "(no words)\n");
  for (const t of r.tests) {
    if (t.status === "pass") {
      print(`PASS     ${t.word}\n`);
      continue;
    }
    if (t.status === "pending") {
      print(`PENDING  ${t.word}  (word has no body yet)\n`);
      continue;
    }
    print(`FAIL     ${t.word}\n    expected: ${t.expected.join(" ")}\n`, "err");
    if (t.actual) print(`    actual:   ${t.actual.join(" ")}\n`, "err");
    if (t.trap) print(`    trap in \`${t.trap.word}\`: ${t.trap.message}\n`, "err");
  }
  if (r.tested) {
    const n = (s) => r.tests.filter((t) => t.status === s).length;
    print(`${n("pass")} passed, ${n("fail")} failed, ${n("pending")} pending\n`);
  }
  if (r.trap) print(r.trap.word ? `trap in \`${r.trap.word}\`: ${r.trap.message}\n` : `trap: ${r.trap.message}\n`, "err");
  for (const t of r.processTraps ?? []) print(`trap in \`${t.word ?? "?"}\` (process ${t.pid}): ${t.message}\n`, "err");
  const s = r.stack;
  // A struct or union value (`point{..}`, `option.some{..}`) gets a line of
  // its own, as in `wack repl`.
  if (s.some((e) => e.value.endsWith("}") && !e.value.startsWith('"'))) {
    print(`(\n${s.map((e) => `${e.type} ${e.value}\n`).join("")})\n`, "stack");
  } else {
    print(s.length ? `( ${s.map((e) => e.type).join(" ")} ) ${s.map((e) => e.value).join(" ")}\n` : "( )\n", "stack");
  }
}

// The program so far: every chunk that defined, declared, tested, forgot or
// forced something without an error, in order. Lines are not kept. A reload
// replays it into a fresh session.
const SAVED = "wack.program";

// Every chunk entered, oldest first, for Up and Down in the input box.
const HISTORY = "wack.history";
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

// The Storage behind `/local`; with storage blocked, `/local` lives in memory.
function localStorageOrNone() {
  try {
    return window.localStorage ?? undefined;
  } catch {
    return undefined;
  }
}

// The example in a `#code=<base64url>` fragment, as UTF-8 text.
function linkText() {
  const b64 = location.hash.slice("#code=".length).replace(/-/g, "+").replace(/_/g, "/");
  const bytes = Uint8Array.from(atob(b64 + "=".repeat((4 - (b64.length % 4)) % 4)), (c) => c.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}

// Run the example a link carries. It goes into the history but not into the
// saved program: loaded code is not saved.
async function loadLink(repl, history) {
  let failure = null;
  let hasMain = false;
  const list = chunks(linkText());
  for (const text of list) {
    print(text.replace(/^/gm, "> ") + "\n", "echo");
    const r = await repl.step(text);
    render(r);
    if (history.at(-1) !== text) {
      history.push(text);
      history.splice(0, history.length - HISTORY_MAX);
      save(history, HISTORY);
    }
    hasMain ||= r.defined.some((d) => d.name === "main");
    if (!r.ok) {
      failure ??=
        r.diagnostics.find((d) => d.severity === "error")?.message ??
        r.trap?.message ??
        r.tests.find((t) => t.status === "fail")?.word;
    }
  }
  print(
    `loaded ${list.length} chunks from the link; they are not saved${hasMain ? "; type main to run it" : ""}\n`,
    "hint",
  );
  console.log(failure == null ? "link ok" : "link FAIL " + failure);
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
  const compiler = await loadCompiler(await (await fetch("wack_web.wasm")).arrayBuffer());
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
    storage: localStorageOrNone(),
  });
  await repl.ready;
  print("Whackford REPL. Type a definition or a line; Enter runs it.\n", "hint");
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
  const history = loadSaved(HISTORY);
  if (location.hash.startsWith("#code=")) await loadLink(repl, history);
  input.disabled = false;
  input.focus();

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
      if (!r.line && r.listing == null && !r.tested && !r.diagnostics.some((d) => d.severity === "error")) {
        program.push(text);
        save(program);
      }
    } finally {
      busy = false;
    }
  });
}

main().catch((e) => print(`${e}\n`, "err"));
