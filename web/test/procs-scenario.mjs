// The process scenario for the browser REPL, run by node-procs.mjs under
// node 22 and by procs.html in headless Vivaldi. No DOM, no Node APIs. `output()` is the console so far.
export async function procScenario(repl, assert, output) {
  const top = (r) => r.stack[r.stack.length - 1];
  const step = async (text) => {
    const r = await repl.step(text);
    assert.ok(r.ok, `${text}: ${JSON.stringify(r.diagnostics)} ${JSON.stringify(r.trap)} ${JSON.stringify(r.processTraps)}`);
    return r;
  };
  await step(": ichan ( -- chan i32 ) chan.make ;");
  await step(": schan ( -- chan str ) chan.make ;");

  // (1) A producer runs while the line waits to receive.
  let r = await step("ichan :> c  c  [ c 1 chan.send c 2 chan.send c chan.close ] spawn  c chan.recv  c chan.recv  c chan.recv");
  assert.equal(
    JSON.stringify(r.stack.slice(1).map((e) => e.value)),
    JSON.stringify(["option.some{v: 1}", "option.some{v: 2}", "option.none{}"]),
    "received in order, then none",
  );
  await step("drop drop drop drop");

  // (2) A receiver spawned in one step is woken by a send in a later one.
  r = await step('schan :> d  d  [ d chan.recv none: [ "closed" println ] some: [ println ] match ] spawn');
  assert.equal(top(r).type, "chan str");
  await step('dup "later" chan.send');
  assert.ok(output().includes("later\n"), `output: ${JSON.stringify(output())}`);
  await step("drop");

  // (3) A line that can never finish traps; the stack is unchanged.
  r = await repl.step("ichan chan.recv");
  assert.ok(r.trap?.message.startsWith("all processes blocked: [line"), JSON.stringify(r.trap));
  assert.ok(r.trap.message.includes("chan"), r.trap.message);
  assert.equal(r.stack.length, 0, "stack unchanged");

  // (4) alt takes the channel that has a value.
  r = await step("ichan :> a  ichan :> b  [ a 3 chan.send ] spawn  [ b 7 chan.send ] spawn  a recv: [ none: [ 0 ] some: [ ] match ] b recv: [ none: [ 0 ] some: [ ] match ] alt");
  assert.equal(JSON.stringify(top(r)), JSON.stringify({ type: "i32", value: "3" }), "alt");
  await step("drop");

  // (5) Kill a parked receiver through /prog/<pid>/ctl.
  r = await step('schan :> e  e  [ e chan.recv drop "zombie" println ] spawn');
  const before = output().length;
  await step('"/prog" ls drop');
  const listing = output().slice(before);
  const pids = listing.split(/\s+/).map((x) => x.replace(/\/$/, "")).filter((x) => /^\d+$/.test(x)).map(Number);
  const victim = Math.max(...pids);
  assert.ok(pids.includes(0) && victim > 0, `ls /prog: ${JSON.stringify(listing)}`);
  await step(`"kill" "/prog/${victim}/ctl" write-file drop`);
  await step("dup chan.close drop");
  assert.ok(!output().includes("zombie"), "the killed receiver never ran");

  // (6) I/O completes inline: a spawned process that reads /dev/time prints
  // after the line.
  const mark = output().length;
  await step('[ "/dev/time" read-file drop drop "timed" println ] spawn  "first" println');
  const io = output().slice(mark);
  assert.ok(io.includes("first") && io.includes("timed") && io.indexOf("first") < io.indexOf("timed"), JSON.stringify(io));

  // (7) A parked process's captured struct survives garbage collection.
  await step("struct box  v: i32");
  await step(": churn ( i32 -- ) [ drop 1 box.new drop ] times ;");
  await step("ichan :> g  g  42 box.new :> bx  [ g chan.recv drop bx box.v i32.to-str println ] spawn");
  await step("5000000 churn");
  const m2 = output().length;
  await step("dup 0 chan.send drop");
  assert.equal(output().slice(m2), "42\n", "captured struct intact");

  // (8) A timer delivers its value.
  r = await step("7 20 time.after chan.recv");
  assert.equal(top(r).value, "option.some{v: 7}");
  await step("drop");

  // (9) The timer beats a slow sender, and process 0 then waits on a channel
  // a sleeper serves later.
  r = await step("ichan :> slow  0 10 time.after :> timer  [ 30 time.sleep  slow 1 chan.send ] spawn  slow recv: [ drop 0 ] timer recv: [ drop 1 ] alt  slow chan.recv drop");
  assert.equal(top(r).type, "i32");
  assert.equal(top(r).value, "1");
  await step("drop");
}
