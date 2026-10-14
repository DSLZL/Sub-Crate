// Development-only bundling of the same utility packages injected by the pinned backend.
const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '../..');
const { createRequire } = require('node:module');
const fromBackend = createRequire(path.join(root, 'backend/package.json'));
const output = path.join(__dirname, '.generated/script-helpers.js');
fs.mkdirSync(path.dirname(output), { recursive: true });
fromBackend('esbuild').buildSync({
  stdin: {
    contents: `import lodash from 'lodash'; import * as yaml from 'yaml';
      import {Buffer} from 'buffer'; import {Base64} from 'js-base64';
      Object.assign(globalThis, {lodash, yaml, Buffer, b64encode: Base64.encode, b64decode: Base64.decode});`,
    resolveDir: path.join(root, 'backend'),
  },
  bundle: true,
  platform: 'browser',
  format: 'iife',
  outfile: output,
});
console.log(output);
