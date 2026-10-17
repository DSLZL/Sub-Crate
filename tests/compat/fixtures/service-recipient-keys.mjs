// CI oracle for the production CRUD boundary, using the pinned age implementation.
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import * as age from '../../../backend/node_modules/age-encryption/dist/index.js';

if (process.env.GITHUB_ACTIONS !== 'true') {
  throw new Error('Execute service recipient acceptance in GitHub Actions only');
}
const requireAge = createRequire(import.meta.resolve('../../../backend/node_modules/age-encryption/dist/index.js'));
const { bech32, bech32m } = await import(pathToFileURL(requireAge.resolve('@scure/base')).href);
const valid = await Promise.all([
  age.generateX25519Identity().then(age.identityToRecipient),
  age.generateHybridIdentity().then(age.identityToRecipient),
]);
const invalid = [];
for (const key of valid) {
  const { prefix, words } = bech32.decode(key, false);
  const padding = words.slice();
  padding[padding.length - 1] |= 1;
  invalid.push(
    bech32.encode(prefix, padding, false),
    bech32m.encode(prefix, words, false),
    bech32.encode(prefix, bech32.toWords(new Uint8Array(prefix === 'age' ? 31 : 1215)), false),
    key.slice(0, -1) + (key.endsWith('q') ? 'p' : 'q'),
    key.toUpperCase(),
    key.slice(0, prefix.length + 1) + key.slice(prefix.length + 1).replace(/[a-z]/, c => c.toUpperCase()),
  );
}
for (const key of valid) {
  new age.Encrypter().addRecipient(key);
}
for (const key of invalid) {
  // The Sub-Store wrapper additionally rejects unsupported/capitalized prefixes.
  let rejected = !key.startsWith('age1');
  try { new age.Encrypter().addRecipient(key); } catch { rejected = true; }
  if (!rejected) throw new Error('Invalid recipient fixture was accepted');
}
console.log(JSON.stringify({ valid, invalid }));
