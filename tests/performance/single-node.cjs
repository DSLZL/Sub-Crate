// The original project's built public module exports its real ProxyUtils.parse.
const fs = require('node:fs');
const path = require('node:path');
const {createRequire} = require('node:module');
const {pathToFileURL} = require('node:url');
const readline = require('node:readline');

(async () => {
  if (process.env.GITHUB_ACTIONS !== 'true') throw new Error('Actions only');
  const original = path.resolve(process.argv[2], 'backend');
  // The browser ESM distribution retains the original Node runtime's eval(require).
  globalThis.require = createRequire(path.join(original, 'package.json'));
  for (const key of ['log', 'info', 'warn', 'error', 'debug']) console[key] = () => {};
  const {parse} = await import(pathToFileURL(path.join(original, 'dist/proxy-utils.esm.mjs')));
  // Import installs persistent log capture. Exclude that I/O only from the pure parser measurement.
  for (const key of ['log', 'info', 'warn', 'error', 'debug']) console[key] = () => {};
  const fixtures = JSON.parse(fs.readFileSync(process.env.SINGLE_NODE_FIXTURES, 'utf8'));
  const lines = readline.createInterface({input: process.stdin});
  for await (const line of lines) {
    const {case: index, iterations, warmup = 0} = JSON.parse(line);
    const raw = [fixtures[index].raw, fixtures[index].raw + '\n'];
    for (let i = 0; i < warmup; i++) parse(raw[i % 2]);
    const began = process.hrtime.bigint();
    let checksum = 0;
    for (let i = 0; i < iterations; i++) checksum += parse(raw[i % 2]).length;
    const elapsed_ns = Number(process.hrtime.bigint() - began);
    process.stdout.write('SINGLE_NODE ' + JSON.stringify({elapsed_ns, iterations, checksum, outputs: raw.map(parse)}) + '\n');
  }
})().catch(error => { process.stderr.write(String(error.stack) + '\n'); process.exitCode = 1; });
