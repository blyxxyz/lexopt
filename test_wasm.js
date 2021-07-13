#!/usr/bin/env node
// We test on wasm32-unknown-unknown as representative of "exotic" platforms
// where we can't look inside OsStrings. It's just barely possible to check if
// the tests pass. If they fail we get an ugly unhelpful traceback.

const child_process = require("child_process");
const fs = require("fs");

const directory = "target/wasm32-unknown-unknown/debug/deps";

child_process.execSync("cargo clean --target wasm32-unknown-unknown");
child_process.execSync(
    "cargo +1.31 test --lib --target wasm32-unknown-unknown --no-run"
);

let path;
for (let name of fs.readdirSync(directory)) {
    if (name.endsWith(".wasm")) {
        path = `${directory}/${name}`;
    }
}

console.log(`Running ${path}`);
let code = fs.readFileSync(path);
let mod = new WebAssembly.Module(code);
let instance = new WebAssembly.Instance(mod);
process.exit(instance.exports.main());
