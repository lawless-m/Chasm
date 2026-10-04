// The worker side of the browser REPL. It owns the funcref table and every
// step's instance (tables cannot be shared between threads); the memory is
// shared with the main thread, which services the I/O ring. `ring_enter`
// rings the doorbell and blocks on Atomics.wait until the main thread is
// done. No DOM, no Node APIs: `post` and `onMessage` are supplied.

const decoder = new TextDecoder();
// Import name of the refs table (`chasm_core::layout::IMPORT_REFS`).
const L_REFS = "refs";

export function attach(post, onMessage) {
  let memory, L, table;
  // `chasm.refs`: references on the memory data stack, by slot index.
  // Made on the first step that needs it, so engines without WasmGC never see it.
  let refs = null;

  function ring_enter() {
    const cells = new Int32Array(memory.buffer);
    Atomics.store(cells, L.DOORBELL >> 2, 0);
    post({ type: "ring" });
    Atomics.wait(cells, L.DOORBELL >> 2, 0);
  }

  // The message and word a Chasm trap left behind, or the engine's message.
  function trapInfo(err) {
    const dv = new DataView(memory.buffer);
    const len = dv.getUint32(L.TRAP_MSG_LEN, true);
    if (len === 0) return { message: String(err?.message ?? err), word: null };
    const str = (a, l) =>
      decoder.decode(new Uint8Array(memory.buffer).slice(dv.getUint32(a, true), dv.getUint32(a, true) + dv.getUint32(l, true)));
    const t = { message: str(L.TRAP_MSG_ADDR, L.TRAP_MSG_LEN), word: str(L.TRAP_WORD_ADDR, L.TRAP_WORD_LEN) };
    dv.setUint32(L.TRAP_MSG_LEN, 0, true);
    return t;
  }

  function call(slot) {
    const r = table.get(slot)();
    return r === undefined ? [] : Array.isArray(r) ? r : [r];
  }

  // Read a struct as a plain tree by calling its accessor words through the
  // table: JavaScript cannot read WasmGC struct fields itself.
  // A union's variant comes from its `tag` word, then that variant's
  // fields from its readers. `name` is the type's display name.
  function renderStruct(ref, name, structs, depth) {
    if (ref === null) return null;
    const short = name.split(" ")[0];
    if (depth >= 3) return { name: short, fields: null };
    const layout = structs[name];
    if (layout.kind === "union") {
      const v = layout.variants[table.get(layout.tag)(ref)];
      return { name: `${short}.${v.name}`, fields: readFields(ref, v.fields, structs, depth) };
    }
    return { name: short, fields: readFields(ref, layout.fields, structs, depth) };
  }

  function readFields(ref, layoutFields, structs, depth) {
    return layoutFields.map((f) => {
      const r = table.get(f.get)(ref);
      const vals = r === undefined ? [] : Array.isArray(r) ? r : [r];
      let v;
      if (f.type === "str") {
        const a = vals[0] >>> 0;
        v = decoder.decode(new Uint8Array(memory.buffer).slice(a, a + (vals[1] >>> 0)));
      } else if (f.type === "bytes") {
        v = { bytes: vals[1] >>> 0 };
      } else if (f.type.startsWith("array ")) {
        v = { elements: (f.type.slice(6) in structs ? vals[2] : vals[1]) >>> 0 };
      } else if (f.type.startsWith("[")) {
        v = { quot: vals[0] };
      } else if (f.type in structs) {
        v = renderStruct(vals[0], f.type, structs, depth + 1);
      } else {
        v = vals[0];
      }
      return [f.field, f.type, v];
    });
  }

  onMessage((msg) => {
    if (msg.type === "render") {
      post({ type: "rendered", values: msg.slots.map((s) => renderStruct(refs.get(s.index), s.name, msg.structs, 0)) });
      return;
    }
    if (msg.type === "init") {
      ({ memory, layout: L } = msg);
      table = new WebAssembly.Table({ element: "anyfunc", initial: 0 });
      return;
    }
    if (msg.type !== "run") return;
    try {
      if (table.length < msg.tableSize) table.grow(msg.tableSize - table.length);
      const mod = msg.module && new WebAssembly.Module(msg.module);
      if (msg.refsSize > 0 || (mod && WebAssembly.Module.imports(mod).some((i) => i.name === L_REFS))) {
        refs ??= new WebAssembly.Table({ element: "anyref", initial: 0 });
        if (refs.length < msg.refsSize) refs.grow(msg.refsSize - refs.length);
      }
      if (mod) {
        const chasm = { memory, table, ring_enter };
        if (refs) chasm.refs = refs;
        const instance = new WebAssembly.Instance(mod, { chasm });
        for (const i of msg.installs) table.set(i.slot, instance.exports[i.export]);
      }
    } catch (e) {
      post({ type: "done", error: String(e), trap: null, tests: [] });
      return;
    }
    const tests = msg.tests.map((t) => {
      try {
        return { values: call(t.slot), trap: null };
      } catch (e) {
        return { values: null, trap: trapInfo(e) };
      }
    });
    let trap = null;
    if (msg.line) {
      try {
        call(msg.line.slot);
      } catch (e) {
        trap = trapInfo(e);
      }
    }
    post({ type: "done", trap, tests });
  });
}
