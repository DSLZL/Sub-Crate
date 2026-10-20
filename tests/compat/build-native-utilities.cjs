// The production binary embeds only locked third-party language utilities.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const {createRequire} = require('node:module');
if (process.env.GITHUB_ACTIONS !== 'true') throw new Error('Build native utilities in GitHub Actions only');
const root = path.resolve(__dirname, '../..');
const fromBackend = createRequire(path.join(root, 'backend/package.json'));
function snapshot(pins, contents, globalName, filenames, provenance = {}, keepNames = false) {
const build = fromBackend('esbuild').buildSync({
  absWorkingDir: root,
  stdin: {
    contents,
    resolveDir: path.join(root, 'backend'),
  },
  bundle: true,
  write: false,
  metafile: true,
  platform: 'browser',
  format: 'iife',
  globalName,
  target: 'es2020',
  minify: true,
  keepNames,
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
  if (![pins.get(metadata.name)].flat().includes(metadata.version) || !input.includes('node_modules/')) {
    throw new Error('Production utility bundle imported unapproved source: ' + input);
  }
  const key = metadata.name + '@' + metadata.version;
  const parentDirectory = path.dirname(directory);
  const parentManifest = path.join(parentDirectory, 'package.json');
  if (fs.existsSync(parentManifest)) {
    const parent = JSON.parse(fs.readFileSync(parentManifest, 'utf8'));
    if (parent.name === metadata.name && parent.version === metadata.version) directory = parentDirectory;
  }
  const relative = path.relative(directory, source).split(path.sep).join('/');
  inputs[key + '/' + relative] = sha256(fs.readFileSync(source));
  if (notices.has(key)) continue;
  const license = fs.readdirSync(directory).find(name => /^licen[cs]e(?:\.(?:md|txt))?$/i.test(name));
  if (!license) throw new Error('Missing license text for ' + key);
  let notice = fs.readFileSync(path.join(directory, license), 'utf8').replace(/\r\n/g, '\n').trim();
  if (metadata.name === 'shoutrrr-ts') {
    notice += '\n\n' + fs.readFileSync(path.join(directory, 'THIRD_PARTY_NOTICES.md'), 'utf8').replace(/\r\n/g, '\n').trim();
    notice += '\n\n' + fs.readFileSync(path.join(directory, 'LICENSES/woodpecker-MIT.txt'), 'utf8').replace(/\r\n/g, '\n').trim();
  }
  notices.set(key, notice);
}
if (notices.size !== [...pins.values()].flat().length) throw new Error('Expected every locked utility package');
const bundle = build.outputFiles[0].text;
const licenses = [...notices].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)
  .map(([name, license]) => name + '\n' + license + '\n').join('\n');
const manifest = JSON.stringify({
  ...provenance,
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
for (const [name, contents] of [[filenames[0], bundle], [filenames[1], licenses], [filenames[2], manifest]]) {
  const file = path.join(output, name);
  if (fs.existsSync(file) && !fs.readFileSync(file).equals(Buffer.from(contents))) {
    throw new Error('Locked production utility snapshot changed: ' + name);
  }
  fs.writeFileSync(file, contents);
  fs.writeFileSync(path.join(evidence, name), contents);
}
console.log('Pinned ' + globalName + ': ' + sha256(bundle));
}

snapshot(new Map([['json5', '2.2.3'], ['yaml', '2.9.0']]),
  "import JSON5 from 'json5'; import {parse as parseYaml, stringify as stringifyYaml} from 'yaml'; export {JSON5, parseYaml, stringifyYaml};",
  'SubStoreUtilities', ['utilities.js', 'LICENSES.txt', 'utilities.json']);
snapshot(new Map([
  ['lodash', '4.18.1'], ['buffer', '6.0.3'], ['js-base64', '3.7.8'],
  ['ip-address', '10.3.1'], ['base64-js', '1.5.1'], ['ieee754', '1.2.1'],
]), "import lodash from 'lodash'; import {Buffer} from 'buffer'; import {Base64} from 'js-base64'; import * as ipAddress from 'ip-address'; export {lodash, Buffer, Base64, ipAddress};",
  'SubStoreScriptUtilities', ['script-utilities.js', 'SCRIPT-LICENSES.txt', 'script-utilities.json']);
const fromAge = createRequire(fromBackend.resolve('age-encryption'));
snapshot(new Map([['@noble/hashes', '2.2.0']]),
  'import {md5} from ' + JSON.stringify(fromAge.resolve('@noble/hashes/legacy.js')) + '; ' +
  'import {bytesToHex} from ' + JSON.stringify(fromAge.resolve('@noble/hashes/utils.js')) + '; ' +
  'export const hexMd5 = bytes => bytesToHex(md5(bytes));',
  'SubStoreHashUtilities', ['hash-utilities.js', 'HASH-LICENSES.txt', 'hash-utilities.json']);
const fromCompat = createRequire(path.join(__dirname, 'package.json'));
snapshot(new Map([['@kayahr/text-encoding', '2.2.0'], ['web-streams-polyfill', '4.3.0']]),
  'export {TextEncoder, TextDecoder} from ' + JSON.stringify(fromCompat.resolve('@kayahr/text-encoding/no-encodings')) + '; ' +
  'export {ReadableStream, WritableStream, TransformStream} from ' + JSON.stringify(fromCompat.resolve('web-streams-polyfill')) + ';',
  'SubStoreWebUtilities', ['web-utilities.js', 'WEB-LICENSES.txt', 'web-utilities.json']);
snapshot(new Map([['core-js', '3.49.0']]),
  "export {default as URL} from 'core-js/actual/url'; export {default as URLSearchParams} from 'core-js/actual/url-search-params';",
  'SubStoreUrlUtilities', ['url-utilities.js', 'URL-LICENSES.txt', 'url-utilities.json']);
snapshot(new Map([
  ['age-encryption', '0.3.0'], ['@noble/ciphers', '2.2.0'],
  ['@noble/curves', ['2.0.0', '2.2.0']], ['@noble/hashes', ['2.0.0', '2.2.0']],
  ['@noble/post-quantum', '0.5.4'], ['@scure/base', '2.2.0'],
]), "export {Encrypter, Decrypter, identityToRecipient, generateX25519Identity, generateHybridIdentity, armor} from 'age-encryption';",
  'SubStoreAgeUtilities', ['age-utilities.js', 'AGE-LICENSES.txt', 'age-utilities.json']);
snapshot(new Map([
  ['dns-packet', '5.6.1'], ['@leichtgewicht/ip-codec', '2.0.5'],
  ['buffer', '6.0.3'], ['base64-js', '1.5.1'], ['ieee754', '1.2.1'],
]), "import * as dnsPacket from 'dns-packet'; export {dnsPacket};",
  'SubStoreDnsUtilities', ['dns-utilities.js', 'DNS-LICENSES.txt', 'dns-utilities.json']);
const notificationEntry = path.join(root, 'backend/node_modules/shoutrrr-ts/dist/index.js');
const adapterPath = path.join(__dirname, 'notification-adapter.cjs');
const notification = require(adapterPath)(fs.readFileSync(notificationEntry));
const nativeEntry = path.join(path.dirname(notificationEntry), 'native.js');
fs.writeFileSync(nativeEntry, notification.source);
snapshot(new Map([['shoutrrr-ts', '0.2.0']]),
  'export {send} from ' + JSON.stringify(nativeEntry) + ';',
  'SubStoreNotificationUtilities', ['notification-utilities.js', 'NOTIFICATION-LICENSES.txt', 'notification-utilities.json'],
  {...notification.provenance, adapter_sha256: crypto.createHash('sha256').update(fs.readFileSync(adapterPath).toString('utf8').replace(/\r\n/g, '\n')).digest('hex')});
const mmdbEntry = path.join(root, 'backend/node_modules/@maxmind/geoip2-node/dist/src/readerModel.js');
const mmdbAdapterPath = path.join(__dirname, 'mmdb-adapter.cjs');
const mmdb = require(mmdbAdapterPath)(fs.readFileSync(mmdbEntry));
const nativeMmdbEntry = path.join(path.dirname(mmdbEntry), 'readerModel-native.js');
fs.writeFileSync(nativeMmdbEntry, mmdb.source);
snapshot(new Map([['@maxmind/geoip2-node', '5.0.0']]),
  'export {default as ReaderModel} from ' + JSON.stringify(nativeMmdbEntry) + ';',
  'SubStoreMmdbUtilities', ['mmdb-utilities.js', 'MMDB-LICENSES.txt', 'mmdb-utilities.json'],
  {...mmdb.provenance, adapter_sha256: crypto.createHash('sha256').update(fs.readFileSync(mmdbAdapterPath).toString('utf8').replace(/\r\n/g, '\n')).digest('hex')}, true);
