// The struct scenario for the browser REPL, run by node-structs.mjs (node 22,
// CI) and structs.html (headless Vivaldi). No DOM, no Node APIs.
export async function structScenario(repl, assert) {
  const top = (r) => r.stack[r.stack.length - 1];
  let r = await repl.step("struct point  x: i32  y: f64");
  assert.ok(r.ok, JSON.stringify(r.diagnostics));
  assert.equal(r.defined.length, 5, "five generated words");

  r = await repl.step("7 2.5 point.new");
  assert.equal(JSON.stringify(r.stack), JSON.stringify([{ type: "point", value: "point{x: 7, y: 2.5}" }]), "echo");

  r = await repl.step("dup point.x");
  assert.equal(JSON.stringify(top(r)), JSON.stringify({ type: "i32", value: "7" }), "field read");

  r = await repl.step("drop dup 9 point.x!");
  assert.equal(top(r).value, "point{x: 9, y: 2.5}", "field write");

  r = await repl.step("1 0 i32.div_s");
  assert.ok(r.trap, "division traps");
  assert.equal(top(r).value, "point{x: 9, y: 2.5}", "stack kept after a trap");

  r = await repl.step("drop 2 array.new ( array point )");
  assert.equal(JSON.stringify(top(r)), JSON.stringify({ type: "array point", value: "<2 elements>" }), "struct array");

  await repl.step("dup 0 1 1.0 point.new array.at!");
  r = await repl.step("dup 0 array.at");
  assert.equal(top(r).value, "point{x: 1, y: 1.0}", "element read");

  r = await repl.step("drop 1 1 array.slice");
  assert.equal(top(r).value, "<1 elements>", "slice");

  await repl.step(": churn ( i32 -- ) [ drop 1 2.5 point.new drop ] times ;");
  r = await repl.step("5000000 churn");
  assert.ok(r.ok, "churn");
  assert.equal(top(r).value, "<1 elements>", "stack kept through GC");

  await repl.step("struct seg  a: point  b: point");
  r = await repl.step("drop 0 0.0 point.new 1 1.0 point.new seg.new");
  assert.equal(top(r).value, "seg{a: point{x: 0, y: 0.0}, b: point{x: 1, y: 1.0}}", "nested");

  await repl.step("struct bag  items: array point");
  r = await repl.step("drop 2 array.new ( array point ) bag.new");
  assert.equal(top(r).value, "bag{items: <2 elements>}", "array field");

  r = await repl.step("test point.x : 3 0.0 point.new point.x -> 3");
  assert.equal(r.tests.length, 1);
  assert.equal(r.tests[0].status, "pass", "test with a struct");
}
