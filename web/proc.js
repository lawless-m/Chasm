// The process scheduler (M11): processes, channels and who waits on what,
// as a state machine. It makes no wasm calls; the worker resumes the
// processes it names. No DOM, no Node APIs. Opcodes and error codes come
// from the compiler's `layout`.
//
// Channel values never reach the scheduler: a sender pushes its value onto
// the channel's queue in wasm, and the scheduler only records which process
// is waiting for that value to be taken (`queued`), so a send is a
// rendezvous.

// `/prog` handles are numbered from here, apart from the namespace's.
export const PROG_HANDLE_BASE = 0x40000000;

export class Scheduler {
  constructor(L) {
    this.L = L;
    // pid -> { state: "ready" | "running" | "parked" | "io" | "done", wait, slot, closure }
    this.procs = new Map([[0, { state: "running", wait: null }]]);
    this.nextPid = 1;
    this.ready = []; // { pid, result, start }
    // id -> { senders, closed, queued: [pid], receivers: [{ pid, arm }] }
    this.chans = new Map();
    this.nextChan = 1;
    this.progHandles = new Map();
    this.nextProgHandle = PROG_HANDLE_BASE;
    // Called with the pid of each process killed, so the host can drop it.
    this.onKill = null;
  }

  /// Pid 0 runs again: the next test or line of a step.
  startMain() {
    this.procs.set(0, { state: "running", wait: null });
  }

  /// Make process N for a closure; it runs when the host next takes a ready
  /// process.
  spawn(slot, closure) {
    const pid = this.nextPid++;
    this.procs.set(pid, { state: "ready", wait: null, slot, closure });
    this.ready.push({ pid, result: 0, start: true });
    return pid;
  }

  chanMake() {
    const id = this.nextChan++;
    this.chans.set(id, { senders: 1, closed: false, queued: [], receivers: [] });
    return id;
  }

  /// Make `pid` ready to resume with `result`, unless it was killed.
  wake(pid, result) {
    const p = this.procs.get(pid);
    if (!p || p.state === "done") return;
    for (const id of p.wait?.chans ?? []) {
      const c = this.chans.get(id);
      if (c) c.receivers = c.receivers.filter((r) => r.pid !== pid);
    }
    p.state = "ready";
    p.wait = null;
    this.ready.push({ pid, result, start: false });
  }

  park(pid, wait) {
    const p = this.procs.get(pid);
    p.state = "parked";
    p.wait = wait;
    return { park: true };
  }

  /// The result a woken receiver resumes with: 1 or 0 for `chan.recv`,
  /// `2 * arm + bit` for an `alt` arm.
  static bit(r, v) {
    return r.arm === null ? v : 2 * r.arm + v;
  }

  submit(pid, op, a0, a1, a2, mem) {
    const L = this.L;
    if (op === L.OP_CHAN_MAKE) return { result: this.chanMake() };
    if (op === L.OP_ALT) return this.alt(pid, a0, a1, mem);
    const c = this.chans.get(a0);
    if (!c) return { result: L.E_BAD_HANDLE };
    switch (op) {
      case L.OP_CHAN_SENDER:
        c.senders += 1;
        return { result: 0 };
      case L.OP_CHAN_SEND: {
        if (c.closed) return { result: L.E_CLOSED };
        c.queued.push(pid);
        const r = c.receivers.shift();
        if (!r) return this.park(pid, { kind: "send", chans: [a0] });
        // The receiver takes this value: no sender was queued before it.
        c.queued.shift();
        this.wake(r.pid, Scheduler.bit(r, 1));
        return { result: 0 };
      }
      case L.OP_CHAN_RECV:
        if (c.queued.length) {
          this.wake(c.queued.shift(), 0);
          return { result: 1 };
        }
        if (c.closed) return { result: 0 };
        c.receivers.push({ pid, arm: null });
        return this.park(pid, { kind: "recv", chans: [a0] });
      case L.OP_CHAN_CLOSE:
        if (c.senders === 0) return { result: L.E_CLOSED };
        c.senders -= 1;
        if (c.senders === 0) {
          c.closed = true;
          for (const r of [...c.receivers]) this.wake(r.pid, Scheduler.bit(r, 0));
        }
        return { result: 0 };
      default:
        return { result: L.E_NOT_SUPPORTED };
    }
  }

  alt(pid, addr, count, mem) {
    const L = this.L;
    const dv = new DataView(mem.buffer, mem.byteOffset);
    const ids = [];
    for (let i = 0; i < count; i++) ids.push(dv.getInt32(addr + 4 * i, true));
    if (ids.some((id) => !this.chans.has(id))) return { result: L.E_BAD_HANDLE };
    for (let i = 0; i < ids.length; i++) {
      const c = this.chans.get(ids[i]);
      if (c.queued.length) {
        this.wake(c.queued.shift(), 0);
        return { result: 2 * i + 1 };
      }
      if (c.closed) return { result: 2 * i };
    }
    ids.forEach((id, i) => this.chans.get(id).receivers.push({ pid, arm: i }));
    return this.park(pid, { kind: "alt", chans: ids });
  }

  /// The next ready process and the result it resumes with (`start` for a
  /// process not yet started), in FIFO order, or null.
  next() {
    const r = this.ready.shift();
    if (!r) return null;
    this.procs.get(r.pid).state = "running";
    return r;
  }

  exit(pid) {
    const p = this.procs.get(pid);
    if (p) {
      p.state = "done";
      p.wait = null;
    }
  }

  /// Stop a process: it leaves every wait list and is never resumed. A value
  /// it was sending stays queued and is still delivered.
  kill(pid) {
    const p = this.procs.get(pid);
    if (!p || p.state === "done") return false;
    for (const c of this.chans.values()) c.receivers = c.receivers.filter((r) => r.pid !== pid);
    this.ready = this.ready.filter((r) => r.pid !== pid);
    p.state = "done";
    p.wait = null;
    this.onKill?.(pid);
    return true;
  }

  // ---- `/prog`: the live processes, and `/prog/<pid>/ctl` to kill one.

  progOpen(path, mode) {
    const L = this.L;
    if (path === "/prog" || path === "/prog/") {
      if (mode !== L.MODE_READ) return L.E_PERMISSION;
      const enc = new TextEncoder();
      const records = this.live().map((pid) => {
        const name = enc.encode(String(pid));
        const rec = new Uint8Array(13 + name.length);
        const dv = new DataView(rec.buffer);
        dv.setUint32(0, name.length, true);
        rec.set(name, 4);
        rec[12 + name.length] = 1; // a directory; size 0
        return rec;
      });
      return this.progAdd({ kind: "dir", records });
    }
    const m = /^\/prog\/(\d+)\/ctl$/.exec(path);
    if (!m) return L.E_NOT_FOUND;
    const pid = Number(m[1]);
    const p = this.procs.get(pid);
    if (!p || p.state === "done") return L.E_NOT_FOUND;
    return this.progAdd({ kind: "ctl", pid });
  }

  progAdd(h) {
    const id = this.nextProgHandle++;
    this.progHandles.set(id, h);
    return id;
  }

  progRead(handle, buf) {
    const L = this.L;
    const h = this.progHandles.get(handle);
    if (!h) return L.E_BAD_HANDLE;
    if (h.kind === "ctl") return L.E_PERMISSION;
    let n = 0;
    while (h.records.length && n + h.records[0].length <= buf.length) {
      const rec = h.records.shift();
      buf.set(rec, n);
      n += rec.length;
    }
    return n === 0 && h.records.length ? L.E_IO : n;
  }

  progWrite(handle, buf) {
    const L = this.L;
    const h = this.progHandles.get(handle);
    if (!h) return L.E_BAD_HANDLE;
    if (h.kind !== "ctl") return L.E_PERMISSION;
    const text = new TextDecoder().decode(buf.slice()).replace(/\n$/, "");
    if (text !== "kill") return L.E_MALFORMED;
    this.kill(h.pid);
    return buf.length;
  }

  progClose(handle) {
    return this.progHandles.delete(handle) ? 0 : this.L.E_BAD_HANDLE;
  }

  ioStart(pid) {
    this.procs.get(pid).state = "io";
  }

  ioDone(pid) {
    const p = this.procs.get(pid);
    if (p.state === "io") p.state = "running";
  }

  live() {
    return [...this.procs].filter(([, p]) => p.state !== "done").map(([pid]) => pid);
  }

  /// Null while a process can still run (one is ready, running or waiting on
  /// I/O); otherwise the message naming what each parked process waits on.
  blocked(name = (pid) => (pid === 0 ? "main" : `process ${pid}`)) {
    const procs = [...this.procs].sort(([a], [b]) => a - b);
    if (procs.some(([, p]) => ["ready", "running", "io"].includes(p.state))) return null;
    const parts = procs
      .filter(([, p]) => p.state === "parked")
      .map(([pid, p]) => {
        const chans = p.wait.chans.map((id) => `chan ${id}`).join(", ");
        if (p.wait.kind === "recv") return `${name(pid)} waits to receive on ${chans}`;
        if (p.wait.kind === "send") return `${name(pid)} waits to send on ${chans}`;
        return `${name(pid)} waits on ${chans} (alt)`;
      });
    return parts.length ? `all processes blocked: ${parts.join("; ")}` : null;
  }
}
