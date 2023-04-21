#!/usr/bin/env node
// We test on wasm32-unknown-unknown as representative of "exotic" platforms
// where we can't look inside OsStrings. It's just barely possible to check if
// the tests pass. If they fail we get an ugly unhelpful traceback.

const child_process = require("child_process");
const fs = require("fs");

const directory = "target/wasm32-unknown-unknown/debug/deps";

const version = process.argv[2] || "stable";
const clean = "cargo clean --target wasm32-unknown-unknown";
const build = `cargo +${version} test --lib --target wasm32-unknown-unknown --no-run`;

console.log(">", clean);
child_process.execSync(clean);
console.log(">", build);
child_process.execSync(build);

let path;
for (let name of fs.readdirSync(directory)) {
    if (name.endsWith(".wasm")) {
        path = `${directory}/${name}`;
    }
}

console.log(">", path);
let code = fs.readFileSync(path);
let mod = new WebAssembly.Module(code);
let instance = new WebAssembly.Instance(mod);
process.exit(instance.exports.main());
