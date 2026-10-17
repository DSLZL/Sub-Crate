(async () => {
  function assert(condition, message) { if (!condition) throw new Error(message); }
  assert(typeof require === 'undefined', 'require must be absent');
  assert(typeof process === 'undefined', 'process must be absent');
  assert(typeof module === 'undefined', 'module must be absent');
  assert($substore.env.isNode === false && context.env.backend === 'Rust', 'truthful script environment');
  assert(typeof $substore.node === 'undefined', 'Node host must be absent');
  const entropy = crypto.getRandomValues(new Uint8Array(32));
  assert(entropy.length === 32 && entropy.some(byte => byte !== 0), 'native entropy is registered before helper initialization');
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
  const timerResults = [];
  const canceled = setTimeout(() => timerResults.push('canceled'), 1);
  clearTimeout(canceled);
  await new Promise(resolve => setTimeout(value => { timerResults.push(value); resolve(); }, 5, 'ok'));
  assert(typeof canceled === 'number' && JSON.stringify(timerResults) === '["ok"]', 'timer arguments and cancellation');
  // Wrapper forms follow processors/index.js:460, 476, 1516 and 1567; no production core is embedded.
  function full(name, script) {
    return new Function('$arguments', '$options', '$substore', 'lodash', 'ProxyUtils', 'yaml', 'Buffer', 'b64d', 'b64e', 'produceArtifact', 'require', script + '\nreturn ' + name)(
      $arguments, $options, $substore, lodash, ProxyUtils, yaml, Buffer, b64d, b64e, produceArtifact, undefined);
  }
  const inputs = [{name: 'zero', value: 0}, {name: 'false', value: false}];
  const fullOperator = full('operator', `async function operator(proxies, targetPlatform, context) {
    await $substore.wait(1);
    if (!$arguments.onlyFlagIP || targetPlatform !== 'ClashMeta' || context.env.backend !== 'Rust') throw new Error('injection lost');
    return proxies.map(proxy => ({...proxy, name: b64d(b64e(proxy.name)) + '-full'}));
  }`);
  assert(JSON.stringify(await fullOperator(inputs, 'ClashMeta', context)) === JSON.stringify([{name: 'zero-full', value: 0}, {name: 'false-full', value: false}]), 'full operator and injected argument/helper/context values');
  const shortcutOperator = full('operator', `async function operator(input = [], targetPlatform, context) {
    let proxies = input; let list = [];
    for await (let $server of proxies) { await $substore.wait(1); $server = {...$server, name: $server.name + '-short'}; list.push($server); }
    return list;
  }`);
  assert((await shortcutOperator(inputs, 'ClashMeta', context))[1].name === 'false-short', 'shortcut operator async iteration');
  const fullFilter = full('filter', 'function filter(proxies) { return proxies.map(proxy => proxy.value === 0); }');
  assert(JSON.stringify(fullFilter(inputs)) === '[true,false]', 'full filter preserves zero versus false');
  const shortcutFilter = full('filter', `async function filter(input = [], targetPlatform, context) {
    let proxies = input; let list = []; const fn = async ($server) => { await $substore.wait(1); return $server.value === false; };
    for await (let $server of proxies) list.push(await fn($server)); return list;
  }`);
  assert(JSON.stringify(await shortcutFilter(inputs)) === '[false,true]', 'shortcut filter async results');
  const response = {statusCode: 200, headers: {}, body: '', unknown: null};
  const fullTransform = full('transformFunction', 'function transformFunction(res, context) { return {...res, body: context.env.backend}; }');
  assert(fullTransform(response, context).body === 'Rust', 'full response transformer');
  const shortcutTransform = full('transformFunction', `async function transformFunction(res = {}, context) {
    let $res = res; await $substore.wait(1); $res = {...$res, headers: {'x-test': 'ok'}}; return $res;
  }`);
  assert(JSON.stringify(await shortcutTransform(response, context)) === JSON.stringify({...response, headers: {'x-test': 'ok'}}), 'shortcut response transformer and falsy/unknown fields');
  const proxies = await operator([{name: '🇯🇵 fixture', server: '1.2.3.4'}]);
  assert(proxies[0].name === '🇺🇸 fixture', 'unchanged ip-flag-node.js HTTP/regex/Unicode/timer behavior');
  try { await $substore.http.get({url: 'invalid-url'}); throw new Error('Invalid URL unexpectedly succeeded'); }
  catch (error) { assert(!/unexpectedly succeeded/.test(error.message), 'HTTP errors must reject'); }
  const nested = await produceArtifact(async name => {
    await $substore.wait(1);
    return name.toUpperCase();
  });
  assert(nested === 'NESTED ARTIFACT' && nestedCompleted, 'async Rust callback re-entry');
  return JSON.stringify({passed: ['environment', 'Node API absence', 'storage', 'Buffer/Base64', 'lodash', 'YAML', 'timer arguments and cancellation', 'full and shortcut operators', 'full and shortcut filters', 'full and shortcut response transformers', 'existing ip-flag script and outgoing headers', 'HTTP error propagation', 'async callback re-entry']});
})()
