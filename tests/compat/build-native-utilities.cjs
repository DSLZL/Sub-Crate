// The production binary embeds only locked third-party language utilities.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const {createRequire} = require('node:module');
if (process.env.GITHUB_ACTIONS !== 'true') throw new Error('Build native utilities in GitHub Actions only');
const root = path.resolve(__dirname, '../..');
const fromBackend = createRequire(path.join(root, 'backend/package.json'));
const pins = new Map([['json5', '2.2.3'], ['yaml', '2.9.0']]);
const build = fromBackend('esbuild').buildSync({
  absWorkingDir: root,
  stdin: {
    contents: "import JSON5 from 'json5'; import {parse as parseYaml, stringify as stringifyYaml} from 'yaml'; export {JSON5, parseYaml, stringifyYaml};",
    resolveDir: path.join(root, 'backend'),
  },
  bundle: true,
  write: false,
  metafile: true,
  platform: 'browser',
  format: 'iife',
  globalName: 'SubStoreUtilities',
  target: 'es2020',
  minify: true,
  legalComments: 'none',
});
const sha256 = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const notices = new Map();
const inputs = {};
for (const input of Object.keys(build.metafile.inputs).sort()) {
  if (input === '<stdin>') continue;
  const source = fs.realpathSync(path.resolve(root, input));
  let directory = path.dirname(source);
  let metadata;
  while (true) {
    const manifest = path.join(directory, 'package.json');
    metadata = fs.existsSync(manifest) ? JSON.parse(fs.readFileSync(manifest, 'utf8')) : null;
    if (metadata?.name && metadata.version) break;
    const parent = path.dirname(directory);
    if (parent === directory) throw new Error('No package metadata for ' + input);
    directory = parent;
  }
  if (pins.get(metadata.name) !== metadata.version || !input.includes('node_modules/')) {
    throw new Error('Production utility bundle imported unapproved source: ' + input);
  }
  const key = metadata.name + '@' + metadata.version;
  const relative = path.relative(directory, source).split(path.sep).join('/');
  inputs[key + '/' + relative] = sha256(fs.readFileSync(source));
  if (notices.has(key)) continue;
  const license = fs.readdirSync(directory).find(name => /^licen[cs]e(?:\.(?:md|txt))?$/i.test(name));
  if (!license) throw new Error('Missing license text for ' + key);
  notices.set(key, fs.readFileSync(path.join(directory, license), 'utf8').replace(/\r\n/g, '\n').trim());
}
if (notices.size !== pins.size) throw new Error('Expected both locked utility packages');
const bundle = build.outputFiles[0].text;
const licenses = [...notices].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)
  .map(([name, license]) => name + '\n' + license + '\n').join('\n');
const manifest = JSON.stringify({
  packages: Object.fromEntries(pins),
  esbuild: fromBackend('esbuild/package.json').version,
  bundle_sha256: sha256(bundle),
  licenses_sha256: sha256(licenses),
  inputs: Object.fromEntries(Object.entries(inputs).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)),
}, null, 2) + '\n';
const output = path.join(root, 'rust/vendor');
const evidence = path.join(__dirname, 'evidence/native-utilities');
fs.mkdirSync(output, {recursive: true});
fs.mkdirSync(evidence, {recursive: true});
for (const [name, contents] of [['utilities.js', bundle], ['LICENSES.txt', licenses], ['utilities.json', manifest]]) {
  const file = path.join(output, name);
  if (fs.existsSync(file) && !fs.readFileSync(file).equals(Buffer.from(contents))) {
    throw new Error('Locked production utility snapshot changed: ' + name);
  }
  fs.writeFileSync(file, contents);
  fs.writeFileSync(path.join(evidence, name), contents);
}
console.log('Pinned JSON5/YAML utilities: ' + sha256(bundle));
