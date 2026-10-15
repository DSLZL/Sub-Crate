(async () => {
  const passed = [];
  const observations = {};
  function assert(value, message) { if (!value) throw new Error(message); }
  function equal(actual, expected, message) {
    assert(JSON.stringify(actual) === JSON.stringify(expected), `${message}: ${JSON.stringify(actual)}`);
  }
  async function check(id, callback) {
    try { observations[id] = await callback(); }
    catch (error) { throw new Error(`${id}: ${error.message}`); }
    passed.push(id);
  }
  const copy = value => JSON.parse(JSON.stringify(value));
  const inputs = [{name: 'zero', value: 0, unknown: {flag: false, nil: null}}, {name: 'false', value: false}];
  const source = {name: '源🙂', unknown: {nil: null, zero: 0, flag: false}};
  function create(type, script, args = {}, options = {}, context = {}) {
    const processor = processorHost.factories[type](script, 'ClashMeta', args, source, options, context);
    assert(context.source === source && typeof context.env.version === 'string', 'source/context injection');
    assert(context.env.backend === (probeKind === 'node' ? 'Node' : 'Rust'), 'truthful processor environment');
    return processor;
  }
  async function operator(script, input = inputs, args = {}, options = {}, context = {}) {
    return await processorHost.ApplyProcessor(create('Script Operator', script, args, options, context), input);
  }
  async function filter(script, input = inputs) {
    return await processorHost.ApplyProcessor(create('Script Filter', script), input);
  }
  async function transform(script, input) {
    return await processorHost.ApplyResponseTransformer(create('Response Transformer', script), input);
  }

  await check('PC01-full-operator-injection-and-cloning', async () => {
    const before = copy(inputs);
    const result = await operator(`async function operator(proxies, targetPlatform, context) {
      await $substore.wait(1);
      if (targetPlatform !== 'ClashMeta' || context.source.name !== '源🙂' || $options.flag !== false || $arguments.zero !== 0) throw new Error('lost parameters');
      if (context.env.backend !== $arguments.backend || $substore.env.isNode !== $arguments.node) throw new Error('wrong environment');
      if (!$arguments.node && (typeof require !== 'undefined' || typeof process !== 'undefined' || typeof $substore.node !== 'undefined')) throw new Error('Node API leaked');
      if ($arguments.node && typeof require !== 'function') throw new Error('Node oracle changed');
      proxies[0].unknown.flag = true;
      return proxies.map(proxy => ({...proxy, name: b64d(b64e(proxy.name)) + '-' + lodash.get($options, 'suffix')}));
    }`, inputs, {zero: 0, backend: probeKind === 'node' ? 'Node' : 'Rust', node: probeKind === 'node'}, {flag: false, suffix: '全🙂'});
    equal(inputs, before, 'operators clone their input');
    const expected = copy(inputs);
    expected[0].unknown.flag = true;
    expected.forEach(item => { item.name += '-全🙂'; });
    equal(result, expected, 'full operator result');
    return result;
  });

  await check('PC02-shortcut-operator-pending-http', async () => {
    const result = await operator(`const response = await $substore.http.get({url: $arguments.url + '?name=' + $server.name, headers: {'X-Probe': 'processor-shortcut'}});
      if (JSON.parse(response.body).probe !== 'processor-shortcut') throw new Error('HTTP response lost');
      await $substore.wait(1); $server.name += '-快捷🙂';`, inputs, {url: probeUpstream + '/host/echo'});
    equal(result, inputs.map(item => ({...item, name: item.name + '-快捷🙂'})), 'shortcut output');
    return result;
  });

  await check('PC03-full-and-shortcut-filter-truthiness', async () => {
    equal(await filter('async function filter(proxies) { await $substore.wait(1); return proxies.map(p => p.value === 0); }'), [inputs[0]], 'full filter');
    equal(await filter('await $substore.wait(1); return $server.value === false;'), [inputs[1]], 'shortcut filter');
    equal(await filter('$server.value === false;'), [], 'shortcut has no implicit return');
    equal(await filter('function filter() { return ["keep", 0]; }'), [inputs[0]], 'selection uses JS truthiness');
    const mutable = copy(inputs);
    equal(await filter('function filter(proxies) { proxies[0].name = "mutated"; return [true]; }', mutable), [mutable[0]], 'filter aliases input');
    assert(mutable[0].name === 'mutated', 'filters do not clone');
    return {full: [inputs[0]], shortcut: [inputs[1]], implicit: [], mutation: mutable};
  });

  await check('PC04-operator-fallback-retains-mutation', async () => {
    const result = await operator('function operator(proxies) { proxies[0].name += "-before-error"; throw new Error("operator failed"); }');
    const expected = copy(inputs);
    expected[0].name += '-before-error';
    equal(result, expected, 'declaration-only shortcut retains first attempt mutation');
    return result;
  });

  await check('PC05-operator-return-truthiness', async () => {
    for (const value of ['undefined', 'null', 'false', '0', '""']) {
      const result = await operator(`function operator(proxies) { proxies[0].name = 'retained'; return ${value}; }`);
      equal(result, [{...inputs[0], name: 'retained'}, inputs[1]], 'falsy return retains mutated clone');
    }
    equal(await operator('function operator() { return []; }'), [], 'empty array replaces input');
    equal(await operator('function operator() { return {unknown: false}; }'), {unknown: false}, 'object return retained');
    return {falsy: 'retained', array: [], object: {unknown: false}};
  });

  await check('PC06-user-errors-and-recovery', async () => {
    const failures = [];
    for (const [type, apply] of [['脚本操作', () => operator('throw new Error("fixture broken");')],
      ['脚本过滤', () => filter('throw new Error("fixture broken");')],
      ['响应修改', () => transform('throw new Error("fixture broken");', {body: ''})]]) {
      try { await apply(); throw new Error('unexpected success'); }
      catch (error) {
        equal(error.message, `${type} 执行失败 fixture broken`, 'same user error in both paths');
        failures.push(error.message);
      }
    }
    equal(await operator('function operator(proxies) { return proxies; }'), inputs, 'context survives script errors');
    return failures;
  });

  await check('PC07-full-response-transform-and-aliases', async () => {
    const input = {statusCode: 201, headers: {'x-before': 'yes'}, body: '原文🙂\r\n', unknown: false};
    const result = await transform(`async function transformFunction(res, context) {
      await $substore.wait(1); res.headers['x-mutated'] = context.source.name;
      return {...res, body: b64d(b64e(res.body)) + 'after\\n'};
    }`, input);
    assert(input.headers['x-mutated'] === source.name, 'response transformer shares its input');
    equal(result, {...input, body: input.body + 'after\n'}, 'full transform result');
    for (const value of ['undefined', 'null', 'false', '0', '""']) {
      const response = {body: '', unknown: null};
      assert(await transform(`function transformFunction(res) { res.body = 'mutated'; return ${value}; }`, response) === response, 'falsy transform preserves identity');
      assert(response.body === 'mutated', 'falsy transform preserves mutation');
    }
    return result;
  });

  await check('PC08-shortcut-response-pending-http', async () => {
    const response = {statusCode: 200, headers: {}, body: '订阅🙂\r\n', zero: 0};
    const result = await transform(`const remote = await $substore.http.post({url: ${JSON.stringify(probeUpstream + '/host/echo')}, body: Buffer.from($res.body), headers: {'X-Probe': 'processor-response'}});
      $res.headers['x-bytes'] = String(JSON.parse(remote.body).body.length);
      await $substore.wait(1); $res.body += '快捷\\n';`, response);
    equal(result, {statusCode: 200, headers: {'x-bytes': '12'}, body: '订阅🙂\r\n快捷\n', zero: 0}, 'shortcut response result');
    assert(result === response, 'shortcut response retains identity');
    return result;
  });

  await check('PC09-file-shortcut-and-local-options', async () => {
    const input = {$content: '订阅🙂\r\n', $files: [{name: 'a', unknown: false}], $options: {flag: false}, $file: {type: 'text', name: 'fixture'}, omitted: null};
    const result = await operator(`if ($options.flag !== false || $arguments.zero !== 0 || context.source.name !== '源🙂') throw new Error('file injections');
      await $substore.wait(1); $content += 'line\\n'; $files[0].unknown = 0;`, input, {zero: 0}, {flag: true});
    equal(result, {$content: '订阅🙂\r\nline\n', $files: [{name: 'a', unknown: 0}], $options: {flag: false}, $file: input.$file}, 'file shortcut emits only its four bindings');
    assert(input.$files[0].unknown === false && input.$content === '订阅🙂\r\n', 'file input cloned');
    return result;
  });

  await check('PC10-mihomo-yaml-patch-branches', async () => {
    const result = await operator('"+proxies": [{name: front}]\n"proxy-groups+": [{name: back}]\n"dns!": {enable: false}\n"<a.b>": 0\n',
      {$file: {type: 'mihomoConfig'}, $content: 'proxies: [{name: old}]\nproxy-groups: [{name: first}]\ndns: {enable: true, old: true}\n', unknown: false});
    const parsed = processorHost.YAML.safeLoad(result.$content);
    equal(parsed, {proxies: [{name: 'front'}, {name: 'old'}], 'proxy-groups': [{name: 'first'}, {name: 'back'}], dns: {enable: false}, '<a.b>': 0}, 'source deep merge retains brackets on scalar keys');
    assert(result.unknown === false, 'YAML path retains unknown input');
    return result;
  });

  await check('PC11-mihomo-async-main-and-legacy-type', async () => {
    const result = [];
    for (const type of ['mihomoConfig', 'mihomoProfile']) {
      const output = await operator('async function main(config) { await $substore.wait(1); config.unknown = false; config["名称"] = "配置🙂"; return config; }',
        {$file: {type}, $content: 'port: 7890\n', $options: {zero: 0}});
      equal(processorHost.YAML.safeLoad(output.$content), {port: 7890, unknown: false, '名称': '配置🙂'}, 'main output');
      result.push(output);
    }
    return result;
  });

  await check('PC12-nested-rust-reentry-pending-processor', async () => {
    const payload = {nodes: inputs, unknown: {nil: null, zero: 0, flag: false}};
    const value = JSON.parse(await probeReenter(JSON.stringify(payload), async raw => {
      const outer = JSON.parse(raw);
      const first = await operator('async function operator(proxies) { await $substore.wait(1); return proxies.map(p => ({...p, name: p.name + "-outer"})); }', outer.nodes);
      return await probeReenter(JSON.stringify(first), async nested => {
        const second = await operator(`async function operator(proxies) {
          const response = await $substore.http.post({url: $arguments.url, body: '嵌套🙂\\r\\n', headers: {'X-Probe': 'processor-nested'}});
          if (JSON.parse(response.body).probe !== 'processor-nested') throw new Error('nested HTTP');
          await $substore.wait(1); return proxies.map(p => ({...p, name: p.name + '-inner'}));
        }`, JSON.parse(nested), {url: probeUpstream + '/host/echo'});
        return JSON.stringify({nodes: second, unknown: outer.unknown});
      });
    }));
    equal(value, {...payload, nodes: inputs.map(p => ({...p, name: p.name + '-outer-inner'}))}, 'nested processor and pending HTTP return');
    const concurrent = await Promise.all(['a', 'b'].map(name => probeReenter(name, async raw => {
      const result = await operator('async function operator(proxies) { await $substore.wait(1); return proxies; }', [{name: raw}]);
      return JSON.stringify(result);
    })));
    equal(concurrent.map(JSON.parse), [[{name: 'a'}], [{name: 'b'}]], 'concurrent callbacks retain arguments');
    try { await probeReenter('failure', async () => { throw new Error('nested rejected'); }); throw new Error('unexpected success'); }
    catch (error) { equal(error.message, 'nested rejected', 'nested rejection'); }
    equal(await probeReenter('recovery', async raw => raw), 'recovery', 'callback recovers after rejection');
    return {value, concurrent: concurrent.map(JSON.parse), recovered: true};
  });

  const cache = processorHost.scriptResourceCache;
  await check('PC13-script-cache-defaults-and-ttl-coercion', async () => {
    const now = Date.now();
    const initial = copy($substore.read('settings'));
    cache.revokeAll();
    for (const ttl of [undefined, null, 0, -1, '', false, 'invalid', Infinity]) {
      assert(cache.set('default', false, ttl) === undefined, 'set returns undefined');
      assert(cache.gettime('default') === now + 172800000, 'invalid TTL uses 48 hours');
    }
    for (const [ttl, milliseconds] of [['12.5', 12.5], [true, 1], [20, 20]]) {
      cache.set('explicit', 0, ttl);
      assert(cache.gettime('explicit') === now + milliseconds, 'positive TTL is milliseconds');
    }
    $substore.write({...initial, scriptCacheTtl: '1.5'}, 'settings');
    cache.set('settings', '', NaN);
    assert(cache.gettime('settings') === now + 1500, 'settings TTL is seconds');
    for (const ttl of ['invalid', -2, Infinity]) {
      $substore.write({...initial, scriptCacheTtl: ttl === Infinity ? 'Infinity' : ttl}, 'settings');
      cache.set('settings', null);
      assert(cache.gettime('settings') === now + 172800000, 'invalid settings fallback');
    }
    $substore.write(initial, 'settings');
    cache.revokeAll();
    return {defaultMs: 172800000, explicitMs: [12.5, 1, 20], settingsMs: 1500};
  });

  await check('PC14-script-cache-expiry-removal-and-prefix', async () => {
    const start = probeNow;
    cache.set('zero', 0, 10);
    cache.set('false', false, 10);
    cache.set('nil', null, 10);
    cache.set('empty', '', 10);
    probeNow += 10;
    equal([cache.get('zero'), cache.get('false'), cache.get('nil'), cache.get('empty')], [0, false, null, ''], 'inclusive expiry retains falsy values');
    assert(cache.gettime('zero') === start + 10, 'gettime inclusive boundary');
    assert(cache.get('zero', 1) === null && cache.resourceCache.zero !== undefined, 'future threshold does not remove');
    probeNow += 1;
    assert(cache.gettime('zero') === null && cache.get('zero', 0, true) === null, 'expired remove');
    assert(!Object.hasOwn(cache.resourceCache, 'zero'), 'remove persists deletion');
    cache.set('prefix-a', {unknown: false}, 20);
    cache.set('other-a', '保留🙂\r\n', 20);
    cache._cleanup('prefix-', 21);
    assert(!Object.hasOwn(cache.resourceCache, 'prefix-a') && cache.get('other-a') === '保留🙂\r\n', 'prefix cleanup threshold');
    cache._cleanup();
    equal(Object.keys(cache.resourceCache), ['other-a'], 'expired keys cleaned');
    assert(cache.get('missing') === null && cache.gettime('missing') === null, 'missing cache results');
    probeNow = start;
    cache.revokeAll();
    return {inclusive: [0, false, null, ''], missing: null, prefixRetained: '保留🙂\r\n'};
  });

  await check('PC15-script-cache-root-persistence-and-aliases', async () => {
    const value = {unknown: {nil: null, zero: 0, flag: false, empty: ''}, text: '订阅🙂\r\n', script: "require('node:fs')"};
    cache.set('restart', value, 60000);
    assert(cache.get('restart') === value, 'cache retains value aliases');
    value.unknown.zero = false;
    cache._persist();
    equal(JSON.parse($substore.read('#sub-store-cached-script-resource')), cache.resourceCache, 'public root contains JSON string');
    assert(!Object.hasOwn($substore.cache, '#sub-store-cached-script-resource'), 'script cache belongs to root');
    assert(typeof $substore.root['sub-store-cached-script-resource'] === 'string', 'serialized cache is not an object');
    return copy(cache.resourceCache);
  });

  await check('PC16-project-yaml-parse-merge-tags-and-errors', async () => {
    const yaml = processorHost.YAML;
    const merged = yaml.safeLoad('base: &base {unknown: false, zero: 0}\nchild: {<<: *base, text: "订阅🙂"}\n');
    equal(merged, {base: {unknown: false, zero: 0}, child: {unknown: false, zero: 0, text: '订阅🙂'}}, 'merge enabled by default');
    equal(yaml.safeLoad('port: !<str> 0123\n'), {port: '0123'}, 'legacy string tag retry');
    assert(yaml.load('nil: null\n').nil === null && yaml.parse('false') === false, 'public parse aliases');
    let rejected = false;
    try { yaml.safeLoad('x: 1\nx: 2\n'); } catch (error) { rejected = error.name === 'YAMLParseError'; }
    assert(rejected, 'duplicate keys reject');
    return merged;
  });

  await check('PC17-project-yaml-dump-options-and-json-boundary', async () => {
    const yaml = processorHost.YAML;
    const result = yaml.safeDump({z: ['订阅🙂'], a: {flag: false, zero: 0}}, {sortKeys: true, forceQuotes: true, quotingType: "'", noArrayIndent: true, lineWidth: -1});
    equal(result, "'a':\n  'flag': false\n  'zero': 0\n'z':\n- '订阅🙂'\n", 'legacy option aliases');
    const shared = {nil: null};
    const serialized = yaml.dump({a: shared, b: shared, missing: undefined}, {noRefs: true});
    assert(!serialized.includes('&') && !serialized.includes('*') && !serialized.includes('missing'), 'JSON boundary and no references');
    let rejected = false;
    const circular = {}; circular.self = circular;
    try { yaml.stringify(circular); } catch (error) { rejected = error instanceof TypeError; }
    assert(rejected, 'circular input rejects');
    return {options: result, serialized};
  });

  await runHelperContracts(check, processorHost, operator, assert, equal);
  await runDnsContracts(check, processorHost, operator, assert, equal);
  await runNativeDnsContracts(check, processorHost, operator, assert, equal);
  $substore.persistCache();
  return {passed, observations, storage: {cache: $substore.cache, root: $substore.root}, resourceCache: cache.resourceCache,
    headersCache: processorHost.headersResourceCache.resourceCache, dnsCache: processorHost.resourceCache.resourceCache};
})()
