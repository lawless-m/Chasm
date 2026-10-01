// The worker side of the browser REPL. It owns the funcref table and every
// step's instance (tables cannot be shared between threads); the memory is
// shared with the main thread, which services the I/O ring. `ring_enter`
// rings the doorbell and blocks on Atomics.wait until the main thread is
// done. No DOM, no Node APIs: `post` and `onMessage` are supplied.

const decoder = new TextDecoder();

export function attach(post, onMessage) {
  let memory, L, table;

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

  onMessage((msg) => {
    if (msg.type === "init") {
      ({ memory, layout: L } = msg);
      table = new WebAssembly.Table({ element: "anyfunc", initial: 0 });
      return;
    }
    if (msg.type !== "run") return;
    try {
      if (table.length < msg.tableSize) table.grow(msg.tableSize - table.length);
      if (msg.module) {
        const instance = new WebAssembly.Instance(new WebAssembly.Module(msg.module), {
          chasm: { memory, table, ring_enter },
        });
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
