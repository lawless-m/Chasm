// The browser host's side of the I/O ring: the same semantics as
// `service_ring` in crates/runtime/src/lib.rs, over a small namespace.
// All layout numbers come from the compiler's `layout`. No DOM, no Node APIs.

const decoder = new TextDecoder();

// TextDecoder refuses views of a SharedArrayBuffer, so decode a copy.
const decode = (u8) => decoder.decode(u8.slice());

/// The browser namespace: `/dev/cons` (output only; reads see end of
/// input) and `/dev/time`. Everything else is not found.
export class Namespace {
  constructor(layout, { onOutput }) {
    this.L = layout;
    this.onOutput = onOutput;
    this.handles = new Map();
    this.next = 3;
  }

  add(h) {
    const id = this.next++;
    this.handles.set(id, h);
    return id;
  }

  open(path, mode) {
    const L = this.L;
    if (mode < L.MODE_READ || mode > L.MODE_READ_WRITE) return L.E_NOT_SUPPORTED;
    if (path === "/dev/cons") return this.add({ kind: "cons" });
    if (path === "/dev/time") {
      return mode === L.MODE_READ ? this.add({ kind: "time", done: false }) : L.E_PERMISSION;
    }
    return L.E_NOT_FOUND;
  }

  read(handle, buf) {
    const h = this.handles.get(handle);
    if (!h) return this.L.E_BAD_HANDLE;
    if (h.kind === "cons") return 0;
    if (h.done) return 0;
    if (buf.length < 8) return this.L.E_IO;
    const ns = BigInt(Date.now()) * 1000000n;
    new DataView(buf.buffer, buf.byteOffset, 8).setBigUint64(0, ns, true);
    h.done = true;
    return 8;
  }

  write(handle, buf) {
    const h = this.handles.get(handle);
    if (!h) return this.L.E_BAD_HANDLE;
    if (h.kind !== "cons") return this.L.E_PERMISSION;
    this.onOutput(decode(buf));
    return buf.length;
  }

  close(handle) {
    return this.handles.delete(handle) ? 0 : this.L.E_BAD_HANDLE;
  }
}

/// Process every pending submission and post its completion.
export function serviceRing(memory, L, host) {
  const i32 = new Int32Array(memory.buffer);
  const dv = new DataView(memory.buffer);
  const u8 = new Uint8Array(memory.buffer);
  const load = (addr) => Atomics.load(i32, addr >> 2) >>> 0;
  const store = (addr, v) => Atomics.store(i32, addr >> 2, v | 0);
  const range = (addr, len) =>
    addr < 0 || len < 0 || addr + len > u8.length ? null : u8.subarray(addr, addr + len);
  for (;;) {
    const head = load(L.SQ_HEAD);
    const tail = load(L.SQ_TAIL);
    if (head === tail) return;
    const e = L.SQ_BASE + (head % L.RING_ENTRIES) * L.SQE_SIZE;
    const op = dv.getInt32(e + L.SQE_OP, true);
    const user = dv.getUint32(e + L.SQE_USER, true);
    const a0 = dv.getInt32(e + L.SQE_A0, true);
    const a1 = dv.getInt32(e + L.SQE_A1, true);
    const a2 = dv.getInt32(e + L.SQE_A2, true);
    let result;
    if (op === L.OP_OPEN) {
      const r = range(a0, a1);
      result = r ? host.open(decode(r), a2) : L.E_IO;
    } else if (op === L.OP_READ || op === L.OP_WRITE) {
      const r = range(a1, a2);
      result = !r ? L.E_IO : op === L.OP_READ ? host.read(a0, r) : host.write(a0, r);
    } else if (op === L.OP_CLOSE) {
      result = host.close(a0);
    } else {
      result = L.E_NOT_SUPPORTED;
    }
    const ctail = load(L.CQ_TAIL);
    const c = L.CQ_BASE + (ctail % L.RING_ENTRIES) * L.CQE_SIZE;
    dv.setUint32(c + L.CQE_USER, user, true);
    dv.setInt32(c + L.CQE_RESULT, result, true);
    store(L.CQ_TAIL, (ctail + 1) >>> 0);
    store(L.SQ_HEAD, (head + 1) >>> 0);
  }
}
