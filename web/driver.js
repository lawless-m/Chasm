// The main-thread side of the browser REPL: the compiler session, the
// shared memory and the I/O ring. Each step follows the session's host
// contract (crates/core/src/repl.rs): place the literals, have the worker
// install and run the step, commit the stack or restore it after a trap.
// No DOM, no Node APIs: the worker connection is supplied.

import { Namespace, serviceRing } from "./ring.js";

const decoder = new TextDecoder();

// Rust's `{:?}` for floats: shortest text that reads back, with a `.0` on
// whole numbers.
function float(x, single) {
  if (!Number.isFinite(x)) return Number.isNaN(x) ? "NaN" : x > 0 ? "inf" : "-inf";
  let s = String(x);
  if (single) {
    for (let p = 1; p <= 9; p++) {
      const t = x.toPrecision(p);
      if (Math.fround(parseFloat(t)) === x) {
        s = String(parseFloat(t));
        break;
      }
    }
  }
  return /[.eE]/.test(s) ? s : s + ".0";
}


export class Repl {
  constructor({ compiler, memory, worker, onOutput }) {
    this.c = compiler;
    this.L = compiler.layout;
    this.memory = memory;
    this.worker = worker;
    this.host = new Namespace(this.L, { onOutput });
    this.stackTypes = [];
    this.structs = {};
    this.pending = null;
    this.pendingRender = null;
    worker.onMessage((m) => this.message(m));
    const dv = new DataView(memory.buffer);
    dv.setUint32(this.L.HEAP_PTR, this.L.LITERALS_BASE, true);
    dv.setUint32(this.L.DATA_STACK_PTR, this.L.DATA_STACK_BASE, true);
    worker.post({ type: "init", memory, layout: this.L });
    this.ready = this.install(compiler.newSession(true, this.L.LITERALS_BASE));
  }

  message(m) {
    if (m.type === "ring") {
      serviceRing(this.memory, this.L, this.host);
      const cells = new Int32Array(this.memory.buffer);
      Atomics.store(cells, this.L.DOORBELL >> 2, 1);
      Atomics.notify(cells, this.L.DOORBELL >> 2);
    } else if (m.type === "done") {
      const resolve = this.pending;
      this.pending = null;
      resolve(m);
    } else if (m.type === "rendered") {
      const resolve = this.pendingRender;
      this.pendingRender = null;
      resolve(m.values);
    }
  }

  // Wasm values each checker type lowers to: a struct is one reference, an
  // array of structs a view ( ref start len ).
  width(ty) {
    if (ty.startsWith("array ")) return ty.slice(6) in this.structs ? 3 : 2;
    return ty === "str" ? 2 : 1;
  }

  install(step) {
    const L = this.L;
    this.structs = step.structs ?? this.structs;
    const end = step.literal_addr + step.literals.length;
    const have = this.memory.buffer.byteLength;
    if (end > have) this.memory.grow(Math.ceil((end - have) / 65536));
    new Uint8Array(this.memory.buffer).set(step.literals, step.literal_addr);
    new DataView(this.memory.buffer).setUint32(L.HEAP_PTR, (end + 7) & ~7, true);
    return new Promise((resolve) => {
      this.pending = resolve;
      this.worker.post({
        type: "run",
        module: step.module.length ? step.module : null,
        installs: step.installs,
        tableSize: step.table_size,
        refsSize: step.refs_size ?? 0,
        line: step.line ? { slot: step.line.slot } : null,
        tests: step.tests.map((t) => ({ slot: t.slot })),
      });
    });
  }

  async step(text) {
    await this.ready;
    const L = this.L;
    let dv = new DataView(this.memory.buffer);
    const s = this.c.step(text, dv.getUint32(L.HEAP_PTR, true));
    const sp = dv.getUint32(L.DATA_STACK_PTR, true);
    const saved = new Uint8Array(this.memory.buffer).slice(L.DATA_STACK_BASE, sp);
    const done = await this.install(s);
    const diagnostics = [...s.diagnostics];
    if (done.error) {
      diagnostics.push({ code: "E_INTERNAL", severity: "error", message: done.error, location: { file: "", line: 0, column: 0, token: "" } });
    }
    const trap = done.trap;
    if (s.line && !done.error) {
      if (trap) {
        new Uint8Array(this.memory.buffer).set(saved, L.DATA_STACK_BASE);
        dv = new DataView(this.memory.buffer);
        dv.setUint32(L.DATA_STACK_PTR, sp, true);
        this.c.lineDone(false);
      } else {
        this.c.lineDone(true);
        this.stackTypes = s.line.stack_after;
      }
    }
    const tests = s.tests.map((t, i) => this.check(t, done.tests[i]));
    return {
      ok: !diagnostics.some((d) => d.severity === "error") && !trap && tests.every((t) => t.status === "pass"),
      diagnostics,
      defined: s.defined,
      forgotten: s.forgotten,
      tests,
      trap,
      stack: await this.readStack(this.stackTypes),
    };
  }

  // Decode a run of wasm values by checker type.
  decode(types, vals) {
    const out = [];
    let i = 0;
    for (const ty of types) {
      if (ty === "str") {
        const a = vals[i] >>> 0;
        out.push(decoder.decode(new Uint8Array(this.memory.buffer).slice(a, a + (vals[i + 1] >>> 0))));
      } else {
        out.push(vals[i]);
      }
      i += this.width(ty);
    }
    return out;
  }

  check(t, r) {
    const base = { word: t.word, expected: t.expected_text, location: t.location };
    if (!r || r.trap) return { ...base, status: "fail", actual: null, trap: r?.trap ?? { message: "test did not run", word: null } };
    const actual = this.decode(t.types, r.values);
    const same = (ty, e, a, text) => {
      switch (ty) {
        case "i64":
          return BigInt(a) === BigInt(text.replace(/i64$/, ""));
        case "f32":
          return e === null ? Number.isNaN(a) : Math.fround(e) === a;
        case "f64":
          return e === null ? Number.isNaN(a) : e === a;
        default:
          return e === a;
      }
    };
    const pass =
      actual.length === t.expected.length &&
      t.types.every((ty, i) => same(ty, t.expected[i].value, actual[i], t.expected_text[i]));
    return { ...base, status: pass ? "pass" : "fail", actual: t.types.map((ty, i) => this.show(ty, actual[i])), trap: null };
  }

  show(ty, v) {
    switch (ty) {
      case "i64":
        return `${v} i64`;
      case "f32":
        return float(v, true) + "f32";
      case "f64":
        return float(v, false);
      case "str":
        return JSON.stringify(v);
      default:
        return String(v);
    }
  }

  // A struct tree from the worker, as core's `Value` Display renders it.
  showValue(tree, ty) {
    if (tree === null) return "null";
    if (tree.elements !== undefined) return `<${tree.elements} elements>`;
    if (tree.quot !== undefined) return `#${tree.quot}`;
    if (tree.fields === undefined) return this.show(ty, tree);
    if (tree.fields === null) return `${tree.name}{...}`;
    const parts = tree.fields.map(([f, fty, v]) => `${f}: ${v !== null && typeof v === "object" ? this.showValue(v, fty) : v === null ? "null" : this.show(fty, v)}`);
    return `${tree.name}{${parts.join(", ")}}`;
  }

  // Render the memory data stack, as core's `repl::read_stack` does. Struct
  // values live in the worker's refs table, so the worker reads them.
  async readStack(types) {
    const L = this.L;
    const dv = new DataView(this.memory.buffer);
    const slot = (i) => L.DATA_STACK_BASE + i * L.STACK_SLOT;
    const out = [];
    const wanted = [];
    let i = 0;
    for (const ty of types) {
      const a = slot(i);
      let value;
      if (ty in this.structs) {
        wanted.push({ at: out.length, index: i, name: ty });
        value = null;
      } else if (ty.startsWith("array ") && ty.slice(6) in this.structs) value = `<${dv.getUint32(slot(i + 2), true)} elements>`;
      else if (ty === "i64") value = this.show(ty, dv.getBigInt64(a, true));
      else if (ty === "f32") value = this.show(ty, dv.getFloat32(a, true));
      else if (ty === "f64") value = this.show(ty, dv.getFloat64(a, true));
      else if (ty === "str") value = this.show(ty, this.decode(["str"], [dv.getUint32(a, true), dv.getUint32(slot(i + 1), true)])[0]);
      else if (ty.startsWith("array ")) value = `<${dv.getUint32(slot(i + 1), true)} elements>`;
      else if (ty.startsWith("[")) value = `#${dv.getUint32(a, true)}`;
      else value = String(dv.getInt32(a, true));
      out.push({ type: ty, value });
      i += this.width(ty);
    }
    if (wanted.length) {
      const values = await new Promise((resolve) => {
        this.pendingRender = resolve;
        this.worker.post({ type: "render", slots: wanted.map(({ index, name }) => ({ index, name })), structs: this.structs });
      });
      wanted.forEach((w, k) => (out[w.at].value = this.showValue(values[k], w.name)));
    }
    return out;
  }
}
