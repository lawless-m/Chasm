// Run the process examples (examples/pipeline.wack, examples/alt.wack, examples/timeout.wack) in the browser REPL:
// every chunk steps cleanly, every test passes, then `main` prints what it
// should. `makeRepl()` gives a fresh { repl, output } per program, output()
// being the console so far. No DOM, no Node APIs.
import { chunks } from "../driver.js";

export const PROGRAMS = [
  { name: "pipeline", prints: "110" },
  { name: "alt", prints: "5 5\n8" },
  { name: "timeout", prints: "-1\n7" },
];

export async function examplesScenario(makeRepl, assert, fetchText) {
  for (const { name, prints } of PROGRAMS) {
    const { repl, output } = await makeRepl();
    const text = await fetchText(`/examples/${name}.wack`);
    let tests = 0;
    for (const chunk of chunks(text)) {
      const r = await repl.step(chunk);
      const what = `${name}: ${chunk.split("\n")[0]}`;
      assert.ok(r.ok, `${what}: ${JSON.stringify(r.diagnostics)} ${JSON.stringify(r.trap)} ${JSON.stringify(r.tests)}`);
      assert.ok(!r.trap, `${what}: trap ${JSON.stringify(r.trap)}`);
      assert.equal(r.processTraps.length, 0, `${what}: ${JSON.stringify(r.processTraps)}`);
      for (const t of r.tests) assert.equal(t.status, "pass", `${what}: test ${t.word} ${JSON.stringify(t)}`);
      tests += r.tests.length;
    }
    assert.ok(tests > 0, `${name}: no tests ran`);
    const before = output().length;
    const r = await repl.step("main");
    assert.ok(r.ok && !r.trap, `${name}: main ${JSON.stringify(r.trap)}`);
    assert.ok(output().slice(before).includes(prints), `${name}: main printed ${JSON.stringify(output().slice(before))}`);
  }
}
