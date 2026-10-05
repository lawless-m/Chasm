// Check web/compiler.js against the built compiler (web/wack_web.wasm).
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { loadCompiler } from "../compiler.js";

const wasm = readFileSync(new URL("../wack_web.wasm", import.meta.url));
const c = await loadCompiler(wasm);
const hp = c.layout.LITERALS_BASE + 0x10_0000;

const prelude = c.newSession(true, c.layout.LITERALS_BASE);
assert.ok(prelude.ok);
assert.ok(prelude.table_size > 10);
assert.ok(prelude.module.length > 0);
assert.equal(c.layout.E_NOT_FOUND, -1);

const sq = c.step(": sq ( i32 -- i32 ) dup i32.mul ;", hp);
assert.ok(sq.ok);
assert.equal(sq.installs.length, 1);

const line = c.step("3 sq", hp);
assert.deepEqual(line.line.stack_after, ["i32"]);
assert.ok(WebAssembly.validate(line.module));

assert.equal(c.needsMore(": f ( -- )"), true);
assert.equal(c.needsMore("3 4"), false);

console.log("compiler.mjs ok");
