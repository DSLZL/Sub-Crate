(async () => {
  function assert(condition, message) { if (!condition) throw new Error(message); }
  assert(typeof require === 'undefined', 'require must be absent');
  assert(typeof process === 'undefined', 'process must be absent');
  assert(typeof module === 'undefined', 'module must be absent');
  assert($substore.env.isNode === false && context.env.backend === 'Rust', 'truthful script environment');
  assert(typeof $substore.node === 'undefined', 'Node host must be absent');
  try { require('node:fs'); throw new Error('Node access unexpectedly worked'); }
  catch (error) { assert(error instanceof ReferenceError && /require/.test(error.message), 'missing Node API diagnostic'); }
  const value = { unknown: { nil: null, zero: 0, flag: false, empty: '' }, script: "require('node:fs')" };
  assert($substore.write(value, 'fixture'), 'host storage write');
  assert(JSON.stringify($substore.read('fixture')) === JSON.stringify(value), 'lossless storage round trip');
  assert($substore.read('missing') === undefined, 'missing storage value');
  assert(Buffer.from('订阅🙂').toString('base64') === b64encode('订阅🙂'), 'Buffer/Base64 Unicode');
  assert(b64decode(b64encode('订阅🙂')) === '订阅🙂', 'Base64 round trip');
  assert(lodash.get({ a: [{ b: 0 }] }, 'a[0].b') === 0, 'lodash helper');
  assert(yaml.parse(yaml.stringify(value)).unknown.nil === null, 'YAML helper');
  const proxies = await operator([{name: '🇯🇵 fixture', server: '1.2.3.4'}]);
  assert(proxies[0].name === '🇺🇸 fixture', 'unchanged ip-flag-node.js HTTP/regex/Unicode/timer behavior');
  try { await $substore.http.get({url: 'invalid-url'}); throw new Error('Invalid URL unexpectedly succeeded'); }
  catch (error) { assert(!/unexpectedly succeeded/.test(error.message), 'HTTP errors must reject'); }
  const nested = await produceArtifact(async name => {
    await $substore.wait(1);
    return name.toUpperCase();
  });
  assert(nested === 'NESTED ARTIFACT' && nestedCompleted, 'async Rust callback re-entry');
  return JSON.stringify({passed: ['environment', 'Node API absence', 'storage', 'Buffer/Base64', 'lodash', 'YAML', 'existing ip-flag script and outgoing headers', 'HTTP error propagation', 'async callback re-entry']});
})()
