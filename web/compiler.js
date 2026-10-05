// The Whackford compiler for JavaScript: a thin wrapper over the C ABI of
// crates/web (wack_web.wasm). No DOM and no Node APIs, so it runs in the
// browser and under node alike.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

export async function loadCompiler(wasmBytes) {
  const { instance } = await WebAssembly.instantiate(wasmBytes, {});
  const x = instance.exports;

  // Views must be re-created after every call: the compiler's memory may grow.
  const bytes = (ptr, len) => new Uint8Array(x.memory.buffer, ptr, len);
  const string = (ptr, len) => decoder.decode(bytes(ptr, len));

  // Call `f(ptr, len, ...rest)` with `text` copied into compiler memory.
  function withText(text, f, ...rest) {
    const data = encoder.encode(text);
    const ptr = x.wack_alloc(data.length);
    bytes(ptr, data.length).set(data);
    try {
      return f(ptr, data.length, ...rest);
    } finally {
      x.wack_free(ptr, data.length);
    }
  }

  function result() {
    const r = JSON.parse(string(x.wack_result_ptr(), x.wack_result_len()));
    r.module = bytes(x.wack_module_ptr(), x.wack_module_len()).slice();
    r.literals = bytes(x.wack_literals_ptr(), x.wack_literals_len()).slice();
    return r;
  }

  const layout = JSON.parse(string(x.wack_layout_ptr(), x.wack_layout_len()));
  // Error codes travel as u32; they are negative i32 values.
  for (const k of Object.keys(layout)) {
    if (k.startsWith("E_")) layout[k] |= 0;
  }

  return {
    layout,
    newSession(prelude, heapPtr) {
      x.wack_new(prelude ? 1 : 0, heapPtr);
      return result();
    },
    step(text, heapPtr) {
      withText(text, x.wack_step, heapPtr);
      return result();
    },
    lineDone(ok) {
      x.wack_line_done(ok ? 1 : 0);
    },
    needsMore(text) {
      return withText(text, x.wack_needs_more) !== 0;
    },
  };
}
