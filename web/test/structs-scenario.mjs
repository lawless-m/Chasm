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

  r = await repl.step("union shape | circle  r: f64 | rect  w: f64  h: f64 | empty");
  assert.ok(r.ok, JSON.stringify(r.diagnostics));
  assert.equal(r.defined.length, 7, "constructors, tag and readers");
  r = await repl.step("drop 1.5 shape.circle");
  assert.equal(JSON.stringify(top(r)), JSON.stringify({ type: "shape", value: "shape.circle{r: 1.5}" }), "union echo");
  r = await repl.step("drop shape.empty");
  assert.equal(top(r).value, "shape.empty{}", "fieldless variant");
  r = await repl.step("drop 3 option.some");
  assert.equal(JSON.stringify(top(r)), JSON.stringify({ type: "option i32", value: "option.some{v: 3}" }), "generic union");
  r = await repl.step("drop 2.0 3.0 shape.rect circle: [ ] rect: [ f64.mul ] empty: [ 0.0 ] match");
  assert.equal(JSON.stringify(top(r)), JSON.stringify({ type: "f64", value: "6.0" }), "match");
  await repl.step("struct node  v: i32  next: option node");
  r = await repl.step("drop 1 option.none ( i32 option node ) node.new option.some 2 swap node.new");
  assert.equal(top(r).value, "node{v: 2, next: option.some{v: node{v: 1, next: option{...}}}}", "nested union");

  r = await repl.step("drop vec.make ( vec i32 )");
  assert.ok(r.ok, JSON.stringify(r.diagnostics));
  r = await repl.step("dup 3 vec.push");
  assert.equal(JSON.stringify(top(r)), JSON.stringify({ type: "vec i32", value: "vec{chunks: <32 elements>, count: 1}" }), "vec echo");

  await repl.step(": inc ( i32 -- i32 ) 1 i32.add ;");
  r = await repl.step("drop 'inc");
  assert.equal(JSON.stringify(top(r)), JSON.stringify({ type: "[ i32 -- i32 ]", value: "[ i32 -- i32 ]" }), "function value echo");
  r = await repl.step("5 swap call");
  assert.equal(JSON.stringify(top(r)), JSON.stringify({ type: "i32", value: "6" }), "call across steps");
  await repl.step(": adder ( i32 -- [ i32 -- i32 ] ) :> k [ k i32.add ] ;");
  r = await repl.step("drop 10 adder 5 swap call");
  assert.equal(JSON.stringify(top(r)), JSON.stringify({ type: "i32", value: "15" }), "closure");
  await repl.step("struct op  f: [ i32 -- i32 ]");
  r = await repl.step("drop 'inc op.new");
  assert.equal(top(r).value, "op{f: [ i32 -- i32 ]}", "function value field");
  r = await repl.step("drop 2 array.new ( array [ -- i32 ] )");
  assert.equal(top(r).value, "<2 elements>", "array of function values");
}
