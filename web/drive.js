// The browser twin of `process_ring_enter` and `drive` in
// crates/runtime/src/native.rs: processes by unwind and rewind. Step modules
// are transformed by the compiler; a process that must wait gets no
// completion, `ring_enter` sets the mode cell to unwinding, every transformed
// function on the stack saves its frame into `wack.frames` and returns, and
// the driver keeps the chain. To resume, it puts the chain back, writes the
// completion, sets rewinding and calls the same entry again. While process 0
// runs and nothing is ready, it waits (Atomics.wait on a private cell) until
// the earliest sleeper is due.
// No DOM, no Node APIs.

import { PROG_HANDLE_BASE } from "./proc.js";

export const KILLED = "killed";

const decoder = new TextDecoder();

export class Driver {
  // `frames` and `spawn`: the mutable anyref globals every step module
  // imports. `doorbell()` blocks until the main thread has serviced the
  // entry at SQ_HEAD; `trapInfo(err)` gives `{ message, word }`.
  constructor(L, { memory, table, sched, frames, spawn, doorbell, trapInfo }) {
    Object.assign(this, { L, memory, table, sched, frames, spawn, doorbell, trapInfo });
    this.current = 0;
    this.chains = new Map(); // pid -> saved frame chain
    this.pendingUser = new Map(); // pid -> ring user of its parked submission
    this.waitCell = new Int32Array(new SharedArrayBuffer(4));
    sched.onKill = (pid) => {
      this.chains.delete(pid);
      this.pendingUser.delete(pid);
    };
  }

  dv() {
    return new DataView(this.memory.buffer);
  }

  setMode(m) {
    this.dv().setUint32(this.L.UNWIND_MODE, m, true);
  }

  complete(user, result) {
    const { L } = this;
    const d = this.dv();
    const ct = d.getUint32(L.CQ_TAIL, true);
    const c = L.CQ_BASE + (ct % L.RING_ENTRIES) * L.CQE_SIZE;
    d.setUint32(c + L.CQE_USER, user, true);
    d.setInt32(c + L.CQE_RESULT, result, true);
    d.setUint32(L.CQ_TAIL, (ct + 1) >>> 0, true);
  }

  // `wack.ring_enter`: the one submission `rt.ring` made.
  ringEnter() {
    const { L, sched } = this;
    const d = this.dv();
    const head = d.getUint32(L.SQ_HEAD, true);
    const e = L.SQ_BASE + (head % L.RING_ENTRIES) * L.SQE_SIZE;
    const op = d.getInt32(e + L.SQE_OP, true);
    const user = d.getUint32(e + L.SQE_USER, true);
    const a0 = d.getInt32(e + L.SQE_A0, true);
    const a1 = d.getInt32(e + L.SQE_A1, true);
    const a2 = d.getInt32(e + L.SQE_A2, true);
    const u8 = () => new Uint8Array(this.memory.buffer);
    let path = null;
    if (op === L.OP_OPEN) {
      const p = decoder.decode(u8().slice(a0, a0 + a1));
      if (p === "/prog" || p.startsWith("/prog/")) path = p;
    }
    const prog = path !== null || ((op === L.OP_READ || op === L.OP_WRITE || op === L.OP_CLOSE) && a0 >= PROG_HANDLE_BASE);
    if (!prog && op < L.OP_SPAWN) {
      // Host I/O completes inline: no other process runs meanwhile.
      this.doorbell();
      return;
    }
    d.setUint32(L.SQ_HEAD, (head + 1) >>> 0, true);
    if (op === L.OP_OPEN) return this.complete(user, sched.progOpen(path, a2));
    if (op === L.OP_READ) return this.complete(user, sched.progRead(a0, u8().subarray(a1, a1 + a2)));
    if (op === L.OP_WRITE) {
      const r = sched.progWrite(a0, u8().subarray(a1, a1 + a2));
      // A process that killed itself ends here.
      if (!sched.live().includes(this.current)) throw new Error(KILLED);
      return this.complete(user, r);
    }
    if (op === L.OP_CLOSE) return this.complete(user, sched.progClose(a0));
    if (op === L.OP_SPAWN) {
      const closure = this.spawn.value;
      this.spawn.value = null;
      sched.spawn(a0, closure);
      return this.complete(user, 0);
    }
    const r = sched.submit(this.current, op, a0, a1, a2, u8());
    if (r.park) {
      this.pendingUser.set(this.current, user);
      this.setMode(L.UNWINDING);
    } else {
      this.complete(user, r.result);
    }
  }

  // Run table `slot` (with `args`) as process 0, named `name`, then every
  // process that can run, until process 0 has returned and nothing is ready;
  // processes still parked stay for later steps. Traps in spawned processes
  // go to `processTraps`. Returns `{ values, trap }`.
  drive(slot, name, args, processTraps) {
    const { L, sched, frames, table } = this;
    sched.startMain();
    let values = null;
    let ended0 = false;
    let next = { pid: 0, start: true };
    while (next) {
      const { pid } = next;
      if (!next.start) {
        frames.value = this.chains.get(pid) ?? null;
        this.chains.delete(pid);
        this.complete(this.pendingUser.get(pid) ?? 0, next.result);
        this.pendingUser.delete(pid);
        this.setMode(L.REWINDING);
      }
      this.current = pid;
      let out;
      let ended = false;
      try {
        let v;
        if (pid === 0) {
          v = table.get(slot)(...args);
        } else {
          const p = sched.procs.get(pid);
          v = table.get(p.slot)(p.closure);
        }
        out = v === undefined ? [] : Array.isArray(v) ? v : [v];
      } catch (e) {
        ended = true;
        const killed = e?.message === KILLED;
        if (!(killed && pid > 0)) {
          const t = this.trapInfo(e);
          if (killed) t.word = name;
          this.setMode(L.UNWIND_OFF);
          frames.value = null;
          if (pid > 0) {
            processTraps.push({ pid, word: t.word, message: t.message });
            sched.kill(pid);
          } else {
            sched.kill(0);
            return { values: null, trap: t };
          }
        }
      }
      if (!ended && this.dv().getUint32(L.UNWIND_MODE, true) === L.UNWINDING) {
        this.setMode(L.UNWIND_OFF);
        const chain = frames.value;
        frames.value = null;
        if (sched.live().includes(pid)) this.chains.set(pid, chain);
      } else {
        sched.exit(pid);
        if (pid === 0 && !ended) {
          values = out;
          ended0 = true;
        }
        this.chains.delete(pid);
        this.pendingUser.delete(pid);
      }
      next = sched.next();
      while (!next && !ended0) {
        const ms = sched.sleepFor();
        if (ms === null) break;
        Atomics.wait(this.waitCell, 0, 0, ms);
        next = sched.next();
      }
    }
    if (ended0) return { values, trap: null };
    const message = sched.blocked((pid) => (pid === 0 ? name : `process ${pid}`)) ?? "process 0 never finished";
    sched.kill(0);
    this.chains.delete(0);
    this.pendingUser.delete(0);
    return { values: null, trap: { message, word: name } };
  }
}
