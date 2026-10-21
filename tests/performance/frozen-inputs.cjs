'use strict';
const {createCipheriv, createDecipheriv, randomBytes, scryptSync} = require('node:crypto');
const fs = require('node:fs');

const magic = Buffer.from('SSPERF1\0');
function key(input, salt) {
  // The original input's plain SHA256 is public; it must never be the key.
  return scryptSync(Buffer.concat([Buffer.from('Sub-Store frozen source key\0'), input]), salt, 32);
}
function seal(input, plaintext, run) {
  const salt = randomBytes(16), iv = randomBytes(12);
  const cipher = createCipheriv('aes-256-gcm', key(input, salt), iv);
  cipher.setAAD(Buffer.from(run));
  const encrypted = Buffer.concat([cipher.update(plaintext), cipher.final()]);
  return Buffer.concat([magic, salt, iv, cipher.getAuthTag(), encrypted]);
}
function open(input, encrypted, run) {
  if (encrypted.length < 52 || !encrypted.subarray(0, 8).equals(magic)) throw Error('Invalid capsule');
  const decipher = createDecipheriv('aes-256-gcm', key(input, encrypted.subarray(8, 24)), encrypted.subarray(24, 36));
  decipher.setAAD(Buffer.from(run));
  decipher.setAuthTag(encrypted.subarray(36, 52));
  return Buffer.concat([decipher.update(encrypted.subarray(52)), decipher.final()]);
}
module.exports = {seal, open};
if (require.main === module) {
  try {
    if (process.env.GITHUB_ACTIONS !== 'true' || !process.env.GITHUB_RUN_ID) throw Error('Actions only');
    const operation = {seal, open}[process.argv[2]];
    if (!operation) throw Error('Invalid operation');
    process.stdout.write(operation(fs.readFileSync(process.argv[3]), fs.readFileSync(0), process.env.GITHUB_RUN_ID));
  } catch (_) {
    process.stderr.write('Frozen input validation failed\n');
    process.exitCode = 1;
  }
}
