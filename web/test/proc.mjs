// Check web/proc.js: the scheduler's channel rules, alt, kill and the
// all-processes-blocked message, without wasm.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { loadCompiler } from "../compiler.js";
import { Scheduler } from "../proc.js";

const { layout: L } = await loadCompiler(
  readFileSync(new URL("../wack_web.wasm", import.meta.url)),
);
const mem = new Uint8Array(64);
const ids = (...xs) => {
  xs.forEach((x, i) => new DataView(mem.buffer).setInt32(4 * i, x, true));
  return [0, xs.length];
};
const run = (s) => {
  const r = s.next();
  return r && [r.pid, r.result];
};

// Send parks until a receive takes the value.
{
  const s = new Scheduler(L);
  const c = s.chanMake();
  const p = s.spawn(7, null);
  assert.deepEqual(run(s), [p, 0]);
  assert.deepEqual(s.submit(p, L.OP_CHAN_SEND, c, 0, 0), { park: true });
  assert.equal(s.next(), null);
  assert.deepEqual(s.submit(0, L.OP_CHAN_RECV, c, 0, 0), { result: 1 });
  assert.deepEqual(run(s), [p, 0]);
}

// Receive parks until a send.
{
  const s = new Scheduler(L);
  const c = s.chanMake();
  const p = s.spawn(7, null);
  assert.deepEqual(s.submit(0, L.OP_CHAN_RECV, c, 0, 0), { park: true });
  assert.deepEqual(run(s), [p, 0]);
  assert.deepEqual(s.submit(p, L.OP_CHAN_SEND, c, 0, 0), { result: 0 });
  assert.deepEqual(run(s), [0, 1]);
}

// Two senders: the channel closes when both have closed; then a receive on
// the drained channel returns 0 at once.
{
  const s = new Scheduler(L);
  const c = s.chanMake();
  assert.deepEqual(s.submit(0, L.OP_CHAN_SENDER, c, 0, 0), { result: 0 });
  assert.deepEqual(s.submit(0, L.OP_CHAN_RECV, c, 0, 0), { park: true });
  const p = s.spawn(7, null);
  run(s);
  assert.deepEqual(s.submit(p, L.OP_CHAN_CLOSE, c, 0, 0), { result: 0 });
  assert.equal(s.next(), null, "one sender left: the receiver still waits");
  assert.deepEqual(s.submit(p, L.OP_CHAN_CLOSE, c, 0, 0), { result: 0 });
  assert.deepEqual(run(s), [0, 0]);
  assert.deepEqual(s.submit(0, L.OP_CHAN_RECV, c, 0, 0), { result: 0 });
  assert.deepEqual(s.submit(0, L.OP_CHAN_CLOSE, c, 0, 0), { result: L.E_CLOSED }, "over-close");
  assert.deepEqual(s.submit(0, L.OP_CHAN_SEND, c, 0, 0), { result: L.E_CLOSED }, "send after close");
  assert.deepEqual(s.submit(0, L.OP_CHAN_RECV, 99, 0, 0), { result: L.E_BAD_HANDLE });
}

// A value queued before the close is still received.
{
  const s = new Scheduler(L);
  const c = s.chanMake();
  const p = s.spawn(7, null);
  run(s);
  s.submit(p, L.OP_CHAN_SEND, c, 0, 0);
  assert.deepEqual(s.submit(0, L.OP_CHAN_CLOSE, c, 0, 0), { result: 0 });
  assert.deepEqual(s.submit(0, L.OP_CHAN_RECV, c, 0, 0), { result: 1 });
  assert.deepEqual(s.submit(0, L.OP_CHAN_RECV, c, 0, 0), { result: 0 });
}

// Alt: the lowest ready arm wins at once; otherwise either channel wakes it.
{
  const s = new Scheduler(L);
  const a = s.chanMake();
  const b = s.chanMake();
  const p = s.spawn(7, null);
  run(s);
  s.submit(p, L.OP_CHAN_SEND, b, 0, 0);
  const q = s.spawn(8, null);
  run(s);
  s.submit(q, L.OP_CHAN_SEND, a, 0, 0);
  assert.deepEqual(s.submit(0, L.OP_ALT, ...ids(a, b), 0, mem), { result: 1 }, "arm 0 takes a value");
  assert.deepEqual(run(s), [q, 0]);
  assert.deepEqual(s.submit(0, L.OP_ALT, ...ids(a, b), 0, mem), { result: 3 }, "arm 1 takes a value");
  assert.deepEqual(run(s), [p, 0]);
  assert.deepEqual(s.submit(0, L.OP_ALT, ...ids(a, b), 0, mem), { park: true });
  const r = s.spawn(9, null);
  run(s);
  assert.deepEqual(s.submit(r, L.OP_CHAN_SEND, b, 0, 0), { result: 0 });
  assert.deepEqual(run(s), [0, 3], "woken by b, arm 1");
  assert.equal(s.chans.get(a).receivers.length, 0, "removed from a's waiters");
  assert.deepEqual(s.submit(0, L.OP_ALT, ...ids(a, b), 0, mem), { park: true });
  assert.deepEqual(s.submit(r, L.OP_CHAN_CLOSE, a, 0, 0), { result: 0 });
  assert.deepEqual(run(s), [0, 0], "a closed and drained: arm 0, bit 0");
}

// Kill: a killed receiver leaves the wait lists and a later close does not
// wake it.
{
  const s = new Scheduler(L);
  const c = s.chanMake();
  const p = s.spawn(7, null);
  run(s);
  assert.deepEqual(s.submit(p, L.OP_CHAN_RECV, c, 0, 0), { park: true });
  assert.equal(s.kill(p), true);
  assert.equal(s.kill(p), false);
  assert.equal(s.kill(42), false);
  s.submit(0, L.OP_CHAN_CLOSE, c, 0, 0);
  assert.equal(s.next(), null);
  assert.deepEqual(s.live(), [0]);
}

// Blocked: names each parked process and what it waits on.
{
  const s = new Scheduler(L);
  const a = s.chanMake();
  const b = s.chanMake();
  const p = s.spawn(7, null);
  const q = s.spawn(8, null);
  assert.equal(s.blocked(), null, "processes are ready");
  run(s);
  run(s);
  s.submit(p, L.OP_CHAN_SEND, a, 0, 0);
  s.submit(q, L.OP_ALT, ...ids(a, b), 0, mem);
  assert.equal(s.blocked(), null, "main is running");
  s.submit(0, L.OP_CHAN_RECV, b, 0, 0);
  // q's alt took p's value at once, so p and q are ready again.
  assert.equal(s.blocked(), null);
  run(s);
  run(s);
  s.exit(p);
  s.submit(q, L.OP_CHAN_RECV, a, 0, 0);
  assert.equal(
    s.blocked((pid) => (pid === 0 ? "[line 1]" : `process ${pid}`)),
    "all processes blocked: [line 1] waits to receive on chan 2; process 2 waits to receive on chan 1",
  );
  s.ioStart(0);
  assert.equal(s.blocked(), null, "waiting on I/O is not blocked");
}
{
  const s = new Scheduler(L);
  const a = s.chanMake();
  const b = s.chanMake();
  const p = s.spawn(7, null);
  run(s);
  s.submit(p, L.OP_CHAN_SEND, a, 0, 0);
  s.submit(0, L.OP_ALT, ...ids(b, a), 0, mem);
  run(s);
  s.submit(p, L.OP_CHAN_SEND, a, 0, 0);
  s.submit(0, L.OP_ALT, ...ids(b), 0, mem);
  assert.equal(
    s.blocked(),
    "all processes blocked: main waits on chan 2 (alt); process 1 waits to send on chan 1",
  );
}

// `/prog`: the live pids as directory records; `kill` through ctl.
{
  const s = new Scheduler(L);
  const killed = [];
  s.onKill = (pid) => killed.push(pid);
  const c = s.chanMake();
  const p = s.spawn(7, null);
  run(s);
  s.submit(p, L.OP_CHAN_RECV, c, 0, 0);
  const d = s.progOpen("/prog", L.MODE_READ);
  const buf = new Uint8Array(64);
  const n = s.progRead(d, buf);
  const names = [];
  for (let at = 0; at < n; ) {
    const len = new DataView(buf.buffer).getUint32(at, true);
    names.push(new TextDecoder().decode(buf.subarray(at + 4, at + 4 + len)));
    assert.equal(buf[at + 12 + len], 1, "a directory");
    at += 13 + len;
  }
  assert.deepEqual(names, ["0", String(p)]);
  assert.equal(s.progClose(d), 0);
  assert.equal(s.progOpen("/prog", L.MODE_WRITE), L.E_PERMISSION);
  const ctl = s.progOpen(`/prog/${p}/ctl`, L.MODE_WRITE);
  assert.ok(ctl > 0);
  const bytes = (t) => new TextEncoder().encode(t);
  assert.equal(s.progWrite(ctl, bytes("stop")), L.E_MALFORMED);
  assert.equal(s.progRead(ctl, buf), L.E_PERMISSION);
  assert.equal(s.progWrite(ctl, bytes("kill\n")), 5);
  assert.deepEqual(killed, [p]);
  assert.equal(s.chans.get(c).receivers.length, 0, "the killed receiver left its channel");
  assert.equal(s.progOpen("/prog/9/ctl", L.MODE_WRITE), L.E_NOT_FOUND);
  assert.equal(s.progOpen(`/prog/${p}/ctl`, L.MODE_WRITE), L.E_NOT_FOUND, "finished");
  assert.equal(s.progOpen("/prog/x", L.MODE_READ), L.E_NOT_FOUND);
  assert.equal(s.progClose(12345), L.E_BAD_HANDLE);
}

// Timers, on a fake clock.
{
  let t = 0;
  const started = (s) => {
    const p = s.spawn(7, null);
    assert.deepEqual(run(s), [p, 0]);
    return p;
  };
  {
    const s = new Scheduler(L, () => t);
    const p = started(s);
    assert.deepEqual(s.submit(p, L.OP_SLEEP, 20, 0, 0), { park: true });
    assert.equal(s.next(), null);
    assert.equal(s.sleepFor(), 20);
    t = 19;
    assert.equal(s.next(), null);
    t = 20;
    assert.deepEqual(run(s), [p, 0]);
    assert.equal(s.sleepFor(), null);
  }
  {
    t = 0;
    const s = new Scheduler(L, () => t);
    const [a, b, c, d] = [started(s), started(s), started(s), started(s)];
    s.submit(a, L.OP_SLEEP, 30, 0, 0);
    s.submit(b, L.OP_SLEEP, 10, 0, 0);
    s.submit(c, L.OP_SLEEP, 40, 0, 0);
    s.submit(d, L.OP_SLEEP, 40, 0, 0);
    t = 100;
    assert.deepEqual([run(s), run(s), run(s), run(s)], [[b, 0], [a, 0], [c, 0], [d, 0]]);
    assert.equal(s.next(), null);
  }
  {
    t = 0;
    const s = new Scheduler(L, () => t);
    const c = s.chanMake();
    const p = started(s);
    s.submit(p, L.OP_SLEEP, 10, 0, 0);
    assert.deepEqual(s.submit(0, L.OP_CHAN_RECV, c, 0, 0), { park: true });
    assert.equal(s.blocked(), null, "a sleeper is not blocked");
  }
  {
    t = 0;
    const s = new Scheduler(L, () => t);
    const p = started(s);
    s.submit(p, L.OP_SLEEP, 10, 0, 0);
    assert.ok(s.kill(p));
    assert.equal(s.sleepFor(), null);
    t = 50;
    assert.equal(s.next(), null);
  }
  {
    t = 0;
    const s = new Scheduler(L, () => t);
    const p = started(s);
    s.submit(p, L.OP_SLEEP, -5, 0, 0);
    assert.equal(s.sleepFor(), 0);
    assert.deepEqual(run(s), [p, 0]);
  }
}

console.log("proc.mjs ok");
