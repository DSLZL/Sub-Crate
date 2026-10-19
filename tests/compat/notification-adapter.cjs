// Build-only adaptation of the pinned generic provider utility; no application code.
const crypto = require('node:crypto');
if (process.env.GITHUB_ACTIONS !== 'true') throw new Error('Adapt notification utilities in GitHub Actions only');
const sha256 = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
module.exports = function adapt(bytes) {
  const original = sha256(bytes);
  if (original !== 'a15c04df88e4d8ebc386ef751a8028dd8dbd568a5cecdfc8766a1b8202de611c') {
    throw new Error('Pinned notification utility source changed');
  }
  let source = bytes.toString('utf8').replace(/\r\n/g, '\n');
  function replaceOnce(before, after) {
    if (source.split(before).length !== 2) throw new Error('Notification adapter source boundary changed');
    source = source.replace(before, after);
  }
  for (const line of [
    'import { AsyncLocalStorage } from "node:async_hooks";\n',
    'import { request } from "node:http";\n',
    'import { request as request$1 } from "node:https";\n',
  ]) replaceOnce(line, '');
  const contextStart = source.indexOf('const activeTransport = new AsyncLocalStorage();');
  const contextEnd = source.indexOf('//#endregion', contextStart);
  if (contextStart < 0 || contextEnd < contextStart) throw new Error('Missing notification context boundary');
  replaceOnce(source.slice(contextStart, contextEnd), `
// Each native delivery owns one isolated JS context and one injected transport.
let activeTransport;
async function withTransport(transport, delivery) {
  if (typeof transport !== 'function' || activeTransport) throw new Error('Isolated notification transport is required');
  activeTransport = transport;
  try { return await delivery(); } finally { activeTransport = undefined; }
}
function getTransport() { return activeTransport; }
`);
  const requestStart = source.indexOf('/** Sends methods that Fetch forbids');
  const requestEnd = source.indexOf('function isSameOrSubdomain', requestStart);
  if (requestStart < 0 || requestEnd < requestStart) throw new Error('Missing notification HTTP boundary');
  replaceOnce(source.slice(requestStart, requestEnd), `function requestWithBody() { throw new Error('Injected notification transport is required'); }\n`);
  if (/node:|\bAsyncLocalStorage\b|\brequest\$1\b/.test(source)) throw new Error('Notification utility retained a Node runtime dependency');
  return {source, provenance: {adapted_from: 'shoutrrr-ts@0.2.0/dist/index.js', original_sha256: original}};
};
