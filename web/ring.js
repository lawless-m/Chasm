// The browser host's side of the I/O ring: the same semantics as
// `service_ring` in crates/runtime/src/lib.rs, over a small namespace.
// All layout numbers come from the compiler's `layout`. No DOM, no Node APIs.

const decoder = new TextDecoder();

// TextDecoder refuses views of a SharedArrayBuffer, so decode a copy.
const decode = (u8) => decoder.decode(u8.slice());

// `/local/<name>` is the item `wack/local/<name>` of a Storage (the page's
// localStorage), its bytes kept as a string of char codes 0 to 255.
const LOCAL = "wack/local/";

const toBinary = (u8) => {
  let s = "";
  for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode(...u8.subarray(i, i + 0x8000));
  return s;
};
const fromBinary = (s) => Uint8Array.from(s, (c) => c.charCodeAt(0));

/// A Storage kept in memory, for hosts without localStorage.
export function memoryStorage() {
  const m = new Map();
  return {
    get length() {
      return m.size;
    },
    key: (i) => [...m.keys()][i] ?? null,
    getItem: (k) => (m.has(k) ? m.get(k) : null),
    setItem: (k, v) => void m.set(k, String(v)),
  };
}

/// The browser namespace: `/dev/cons` (output only; reads see end of
/// input), `/dev/time`, `/net/http` and `/net/https` through `fetch`, and
/// `/local`, blobs in a Storage. Everything else is not found.
export class Namespace {
  constructor(layout, { onOutput, storage = memoryStorage() }) {
    this.L = layout;
    this.onOutput = onOutput;
    this.storage = storage;
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
    if (path.startsWith("/net/")) return this.openNet(path.slice(5));
    if (path === "/local" || path === "/local/") {
      return mode === L.MODE_READ ? this.add({ kind: "dir", records: this.localRecords() }) : L.E_PERMISSION;
    }
    if (path.startsWith("/local/")) return this.openLocal(path.slice(7), mode);
    return L.E_NOT_FOUND;
  }

  // One flat directory: a name is a single path segment.
  openLocal(name, mode) {
    const L = this.L;
    if (!name || name.includes("/")) return L.E_NOT_FOUND;
    const key = LOCAL + name;
    const have = this.storage.getItem(key);
    if (have === null && mode === L.MODE_READ) return L.E_NOT_FOUND;
    const data = have === null || mode === L.MODE_WRITE ? new Uint8Array(0) : fromBinary(have);
    const h = { kind: "local", key, mode, data, pos: mode === L.MODE_APPEND ? data.length : 0 };
    if (mode !== L.MODE_READ && !this.save(h)) return L.E_IO;
    return this.add(h);
  }

  save(h) {
    try {
      this.storage.setItem(h.key, toBinary(h.data));
      return true;
    } catch {
      return false; // over quota
    }
  }

  // Directory records for `/local`, by name: u32 name length, name, u64
  // size, u8 is-dir.
  localRecords() {
    const enc = new TextEncoder();
    const names = [];
    for (let i = 0; i < this.storage.length; i++) {
      const k = this.storage.key(i);
      if (k?.startsWith(LOCAL)) names.push(k.slice(LOCAL.length));
    }
    return names.sort().map((n) => {
      const name = enc.encode(n);
      const rec = new Uint8Array(13 + name.length);
      const dv = new DataView(rec.buffer);
      dv.setUint32(0, name.length, true);
      rec.set(name, 4);
      dv.setBigUint64(4 + name.length, BigInt(this.storage.getItem(LOCAL + n).length), true);
      return rec;
    });
  }

  // `/net/http/<host>[:port]/<path>` or `/net/https/...`.
  openNet(rest) {
    const L = this.L;
    const slash = rest.indexOf("/");
    const scheme = slash < 0 ? rest : rest.slice(0, slash);
    if (scheme !== "http" && scheme !== "https") return L.E_NOT_SUPPORTED;
    const after = slash < 0 ? "" : rest.slice(slash + 1);
    const cut = after.indexOf("/");
    const host = cut < 0 ? after : after.slice(0, cut);
    if (!host) return L.E_NOT_FOUND;
    const sub = cut < 0 ? "" : after.slice(cut + 1);
    return this.add({ kind: "http", url: `${scheme}://${host}/${sub}`, written: [], body: null, pos: 0 });
  }

  // Send the request written so far: header lines, an empty line, a body.
  // Resolves to the response body or a negative error code.
  async request(h) {
    const L = this.L;
    const all = new Uint8Array(h.written.reduce((n, c) => n + c.length, 0));
    let at = 0;
    for (const c of h.written) {
      all.set(c, at);
      at += c.length;
    }
    // Browsers drop forbidden header names (Host, Content-Length, ...) on
    // append, as the Fetch standard requires.
    const headers = new Headers();
    let body = new Uint8Array(0);
    if (all.length) {
      let pos = 0;
      for (;;) {
        const nl = all.indexOf(10, pos);
        if (nl < 0) return L.E_MALFORMED;
        let line = decode(all.subarray(pos, nl));
        pos = nl + 1;
        if (line.endsWith("\r")) line = line.slice(0, -1);
        if (line === "") break;
        const colon = line.indexOf(": ");
        if (colon < 0) return L.E_MALFORMED;
        try {
          headers.append(line.slice(0, colon), line.slice(colon + 2));
        } catch {
          return L.E_MALFORMED;
        }
      }
      body = all.slice(pos);
    }
    let resp;
    try {
      resp = await fetch(h.url, body.length ? { method: "POST", headers, body } : { method: "GET", headers });
    } catch {
      return L.E_IO;
    }
    if (resp.status === 404) return L.E_NOT_FOUND;
    if (resp.status === 401 || resp.status === 403) return L.E_PERMISSION;
    if (resp.status < 200 || resp.status > 299) return L.E_IO;
    try {
      return new Uint8Array(await resp.arrayBuffer());
    } catch {
      return L.E_IO;
    }
  }

  read(handle, buf) {
    const h = this.handles.get(handle);
    if (!h) return this.L.E_BAD_HANDLE;
    if (h.kind === "cons") return 0;
    if (h.kind === "http") return this.readHttp(h, buf);
    if (h.kind === "local") {
      if (h.mode === this.L.MODE_WRITE || h.mode === this.L.MODE_APPEND) return this.L.E_PERMISSION;
      const n = Math.min(buf.length, h.data.length - h.pos);
      buf.set(h.data.subarray(h.pos, h.pos + n));
      h.pos += n;
      return n;
    }
    if (h.kind === "dir") {
      // Whole records only.
      let n = 0;
      while (h.records.length && n + h.records[0].length <= buf.length) {
        const rec = h.records.shift();
        buf.set(rec, n);
        n += rec.length;
      }
      return n === 0 && h.records.length ? this.L.E_IO : n;
    }
    if (h.done) return 0;
    if (buf.length < 8) return this.L.E_IO;
    const ns = BigInt(Date.now()) * 1000000n;
    new DataView(buf.buffer, buf.byteOffset, 8).setBigUint64(0, ns, true);
    h.done = true;
    return 8;
  }

  async readHttp(h, buf) {
    if (h.body === null) h.body = await this.request(h);
    if (typeof h.body === "number") return h.body;
    const n = Math.min(buf.length, h.body.length - h.pos);
    buf.set(h.body.subarray(h.pos, h.pos + n));
    h.pos += n;
    return n;
  }

  write(handle, buf) {
    const h = this.handles.get(handle);
    if (!h) return this.L.E_BAD_HANDLE;
    if (h.kind === "http") {
      if (h.body !== null) return this.L.E_PERMISSION;
      h.written.push(buf.slice());
      return buf.length;
    }
    if (h.kind === "local") {
      if (h.mode === this.L.MODE_READ) return this.L.E_PERMISSION;
      const end = h.pos + buf.length;
      if (end > h.data.length) {
        const grown = new Uint8Array(end);
        grown.set(h.data);
        h.data = grown;
      }
      h.data.set(buf, h.pos);
      h.pos = end;
      return this.save(h) ? buf.length : this.L.E_IO;
    }
    if (h.kind !== "cons") return this.L.E_PERMISSION;
    this.onOutput(decode(buf));
    return buf.length;
  }

  close(handle) {
    return this.handles.delete(handle) ? 0 : this.L.E_BAD_HANDLE;
  }
}

/// Perform one submission entry against `host` and return its result,
/// leaving the ring's heads and completions alone. A host call may return a
/// promise (an HTTP request); it is awaited.
export async function serviceEntry(memory, L, host, { op, a0, a1, a2 }) {
  const u8 = new Uint8Array(memory.buffer);
  const range = (addr, len) =>
    addr < 0 || len < 0 || addr + len > u8.length ? null : u8.subarray(addr, addr + len);
  if (op === L.OP_OPEN) {
    const r = range(a0, a1);
    return r ? await host.open(decode(r), a2) : L.E_IO;
  }
  if (op === L.OP_READ || op === L.OP_WRITE) {
    const r = range(a1, a2);
    return !r ? L.E_IO : await (op === L.OP_READ ? host.read(a0, r) : host.write(a0, r));
  }
  if (op === L.OP_CLOSE) return await host.close(a0);
  return L.E_NOT_SUPPORTED;
}

/// Process every pending submission, in order, and post its completion. A
/// host call may return a promise (an HTTP request); it is awaited before
/// the next entry.
export async function serviceRing(memory, L, host) {
  const i32 = new Int32Array(memory.buffer);
  const dv = new DataView(memory.buffer);
  const load = (addr) => Atomics.load(i32, addr >> 2) >>> 0;
  const store = (addr, v) => Atomics.store(i32, addr >> 2, v | 0);
  for (;;) {
    const head = load(L.SQ_HEAD);
    const tail = load(L.SQ_TAIL);
    if (head === tail) return;
    const e = L.SQ_BASE + (head % L.RING_ENTRIES) * L.SQE_SIZE;
    const user = dv.getUint32(e + L.SQE_USER, true);
    const result = await serviceEntry(memory, L, host, {
      op: dv.getInt32(e + L.SQE_OP, true),
      a0: dv.getInt32(e + L.SQE_A0, true),
      a1: dv.getInt32(e + L.SQE_A1, true),
      a2: dv.getInt32(e + L.SQE_A2, true),
    });
    const ctail = load(L.CQ_TAIL);
    const c = L.CQ_BASE + (ctail % L.RING_ENTRIES) * L.CQE_SIZE;
    dv.setUint32(c + L.CQE_USER, user, true);
    dv.setInt32(c + L.CQE_RESULT, result, true);
    store(L.CQ_TAIL, (ctail + 1) >>> 0);
    store(L.SQ_HEAD, (head + 1) >>> 0);
  }
}
