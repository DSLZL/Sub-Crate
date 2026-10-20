globalThis.runHelperContracts = async function(check, host, operator, assert, equal) {
  const utils = host.ProxyUtils;
  const flow = host.flowUtils;
  function errorName(callback) {
    try { callback(); } catch (error) { return error.name; }
    throw new Error('Expected helper rejection');
  }

  await check('PC18-ip-validation-and-coercion', () => {
    const valid4 = ['0.0.0.0', '127.0.0.1', '255.255.255.255'];
    const valid6 = ['::', '::1', '2001:db8::1', '::ffff:192.0.2.1', 'fe80::1%eth0'];
    const invalid = ['01.2.3.4', '256.1.1.1', '1.2.3', '127.0.0.1/24', '[::1]', '2001:::1', ' ::1', '', null, false, 0, undefined];
    assert(valid4.every(ip => utils.isIPv4(ip) && !utils.isIPv6(ip) && utils.isIP(ip)), 'IPv4 boundaries');
    assert(valid6.every(ip => utils.isIPv6(ip) && !utils.isIPv4(ip) && utils.isIP(ip)), 'IPv6 compressed/mapped/zone');
    assert(invalid.every(ip => !utils.isIP(ip)), 'invalid addresses');
    const coerced = {toString: () => '192.0.2.1'};
    assert(utils.isIPv4(coerced) && utils.isIP(['::1']), 'regex string coercion is retained');
    assert(errorName(() => utils.isIP(Symbol('ip'))) === 'TypeError', 'Symbol coercion rejects');
    return {valid4, valid6, invalidCount: invalid.length, coerced: true};
  });

  await check('PC19-address4-construction-subnets-and-bigint', () => {
    const {Address4, AddressError} = utils.ipAddress;
    const ip = new Address4('192.168.1.42/24');
    const result = {form: ip.correctForm(), start: ip.startAddress().correctForm(), end: ip.endAddress().correctForm(),
      bytes: ip.toArray(), integer: ip.bigInt().toString(), member: ip.isInSubnet(new Address4('192.168.1.0/24')),
      outside: ip.isInSubnet(new Address4('192.168.2.0/24')), private: ip.isPrivate(), mask: ip.subnetMask};
    equal(result, {form: '192.168.1.42', start: '192.168.1.0', end: '192.168.1.255', bytes: [192,168,1,42],
      integer: '3232235818', member: true, outside: false, private: true, mask: 24}, 'Address4 values');
    assert(Address4.isValid('192.0.2.1/24') && !Address4.isValid('300.1.1.1'), 'CIDR validation differs from public regex');
    assert(Address4.fromInteger(0xffffffff).correctForm() === '255.255.255.255', 'integer boundary');
    try { new Address4('invalid'); throw new Error('accepted invalid IPv4'); }
    catch (error) { assert(error instanceof AddressError && error.message === 'Invalid IPv4 address.', 'public AddressError identity/message'); }
    return result;
  });

  await check('PC20-address6-construction-mapping-and-url', () => {
    const {Address6} = utils.ipAddress;
    const ip = new Address6('2001:db8::1234/64');
    const url = Address6.fromURL('http://[2001:db8::1]:8080/');
    const result = {form: ip.correctForm(), canonical: ip.canonicalForm(), start: ip.startAddress().correctForm(),
      end: ip.endAddress().correctForm(), mask: ip.subnetMask, mapped: Address6.fromAddress4('192.0.2.1').to4in6(),
      loopback: new Address6('::1').isLoopback(), linkLocal: new Address6('fe80::1').isLinkLocal(),
      integer: Address6.fromBigInt(1n).bigInt().toString(), url: url.address.correctForm(), port: url.port};
    equal(result, {form: '2001:db8::1234', canonical: '2001:0db8:0000:0000:0000:0000:0000:1234', start: '2001:db8::',
      end: '2001:db8::ffff:ffff:ffff:ffff', mask: 64, mapped: '::ffff:192.0.2.1', loopback: true, linkLocal: true,
      integer: '1', url: '2001:db8::1', port: 8080}, 'Address6 values');
    assert(new Address6('fe80::1%eth0').zone === '%eth0' && Address6.isValid('2001:db8::1/64'), 'zone and CIDR');
    assert(errorName(() => new Address6('2001:::1')) === 'AddressError', 'IPv6 parse rejection');
    return result;
  });

  await check('PC21-uuid-shape-and-types', () => {
    const values = ['00000000-0000-0000-0000-000000000000', 'ABCDEFAB-CDEF-FFFF-0000-ABCDEFABCDEF',
      'abcdefabcdefabcdefabcdefabcdefab', '00000000-0000-0000-0000-00000000000g', '', 0, null, undefined,
      {toString: () => '00000000-0000-0000-0000-000000000000'}];
    const result = values.map(utils.isValidUUID);
    equal(result, [true, true, false, false, false, false, false, false, false], 'UUID is a string shape check, not a version check');
    return result;
  });

  await check('PC22-port-selection-boundaries-and-legacy-inputs', () => {
    const original = Math.random;
    const choose = (input, draws) => {
      let index = 0;
      Math.random = () => { assert(index < draws.length, 'unexpected random draw'); return draws[index++]; };
      const value = utils.getRandomPort(input);
      assert(index === draws.length, 'all expected draws consumed');
      return value;
    };
    try {
      const result = [choose('80,443/1000-1002', [0]), choose('80,443/1000-1002', [.4]),
        choose('80,443/1000-1002', [.99, 0]), choose('80,443/1000-1002', [.99, .999]),
        choose(' 8080 ', [0]), choose('', [0]), choose('10-5', [0, 0])];
      equal(result, [80, 443, 1000, 1002, 8080, 0, 10], 'part selection, inclusive range and legacy coercion');
      assert(Number.isNaN(choose('invalid', [0])), 'invalid numeric input yields NaN');
      assert(errorName(() => utils.getRandomPort(80)) === 'TypeError', 'non-string port rejects');
      return result;
    } finally { Math.random = original; }
  });

  await check('PC23-country-flags-iso-and-unicode-removal', () => {
    const names = ['香港 01', 'Tokyo', 'us-01', 'US-01', 'CN2', '内蒙古', '剩余流量', '🇯🇵 mystery', '🇯🇵 美国', 'mystery'];
    const result = names.map(name => ({flag: utils.getFlag(name), iso: utils.getISO(name) ?? null}));
    equal(result, [{flag:'🇭🇰',iso:'HK'}, {flag:'🇯🇵',iso:'JP'}, {flag:'🏴‍☠️',iso:null}, {flag:'🇺🇸',iso:'US'},
      {flag:'🏴‍☠️',iso:null}, {flag:'🇨🇳',iso:'CN'}, {flag:'🏳️‍🌈',iso:'EXP'}, {flag:'🇯🇵',iso:'JP'},
      {flag:'🇺🇸',iso:'US'}, {flag:'🏴‍☠️',iso:null}], 'keyword priority, case-sensitive ISO and existing flags');
    assert(utils.removeFlag(' 🇭🇰 🇯🇵🏴‍☠️🏳️‍🌈 节点🙂 ') === '节点🙂', 'remove full Unicode flag sequences');
    assert(errorName(() => utils.getFlag(null)) === 'TypeError' && errorName(() => utils.removeFlag(0)) === 'TypeError', 'name type errors');
    return result;
  });

  await check('PC24-json5-parsing-reviver-and-own-properties', () => {
    const parsed = utils.JSON5.parse("// source\n{unquoted:'订阅🙂', hex:0x10, plus:+2, trailing:[false,0,null,], infinity:Infinity, nan:NaN, '__proto__':{polluted:true}}", (key, value) => key === 'hex' ? value + 1 : value);
    assert(parsed.unquoted === '订阅🙂' && parsed.hex === 17 && parsed.plus === 2 && parsed.infinity === Infinity && Number.isNaN(parsed.nan), 'JSON5 extensions/reviver');
    equal(parsed.trailing, [false,0,null], 'falsy values retained');
    assert(Object.hasOwn(parsed, '__proto__') && parsed.__proto__.polluted === true && ({}).polluted === undefined, 'own prototype-named property');
    assert(errorName(() => utils.JSON5.parse('{broken:}')) === 'SyntaxError', 'invalid JSON5');
    return {text: parsed.unquoted, integer: parsed.hex, values: parsed.trailing, infinity: parsed.infinity === Infinity, nan: Number.isNaN(parsed.nan)};
  });

  await check('PC25-json5-stringify-options-and-errors', () => {
    const source = {z:'订阅🙂', a:NaN, inf:Infinity, missing:undefined, values:[false,0,null,undefined]};
    const text = utils.JSON5.stringify(source, ['z','a','values'], 2);
    equal(text, "{\n  z: '订阅🙂',\n  a: NaN,\n  values: [\n    false,\n    0,\n    null,\n    null,\n  ],\n}", 'JSON5 formatting and replacer');
    assert(utils.JSON5.stringify(undefined) === undefined && utils.JSON5.stringify({inf:Infinity}) === '{inf:Infinity}', 'undefined and non-JSON numbers');
    const circular = {}; circular.self = circular;
    assert(errorName(() => utils.JSON5.stringify(circular)) === 'TypeError', 'circular JSON5 rejects');
    return text;
  });

  await check('PC26-md5-fixed-ascii-and-unicode-vectors', () => {
    const vectors = [['','d41d8cd98f00b204e9800998ecf8427e'], ['abc','900150983cd24fb0d6963f7d28e17f72'],
      ['message digest','f96b697d7cb7938d525a2f31aaf161d0'], ['订阅🙂\r\n','0cb5f141a0427155bace27275da7b434']];
    const result = vectors.map(([input, expected]) => { const actual = utils.hex_md5(input); assert(actual === expected, 'fixed MD5 vector'); return actual; });
    assert(errorName(() => utils.hex_md5(null)) === 'TypeError', 'non-string MD5 rejects');
    const legacy = ['\ud800', '\udc00', '\ud800x\udc00', '\0', 'a'.repeat(10000)].map(value => utils.hex_md5(value));
    assert(legacy.every(hash => /^[0-9a-f]{32}$/.test(hash)), 'legacy UTF16 and long MD5 hashes');
    assert(legacy[0] !== utils.hex_md5('\ufffd') && legacy[0] !== legacy[1], 'unpaired surrogates keep their original bytes');
    assert(errorName(() => utils.hex_md5()) === 'TypeError', 'missing MD5 input rejects');
    return result.concat(legacy);
  });

  await check('PC27-flow-header-casing-encoding-and-numbers', () => {
    const header = flow.getFlowField({'Subscription-Userinfo':'upload=-1.5e2; download=+2E2; total=1e3; expire=0; reset_day=03',
      'PROFILE-WEB-PAGE-URL':'https://example.invalid/订阅?a=1&b=2', 'Plan-Name':'套餐🙂; A=B'});
    const expected = {expires:0, total:1000, usage:{upload:-150, download:200}, remainingDays:3,
      appUrl:'https://example.invalid/订阅?a=1&b=2', planName:'套餐🙂; A=B'};
    equal(flow.parseFlowHeaders(header), expected, 'encoded header values and numeric forms');
    assert(flow.getFlowField({'subscription-userinfo':0, 'plan-name':false}) === '', 'falsy metadata');
    equal(flow.parseFlowHeaders('download=0; total=0'), {total:0,usage:{upload:0,download:0}}, 'optional fields and absent upload');
    equal(flow.parseFlowHeaders('download=1; total=3; download=99; total=100'), {total:3,usage:{upload:0,download:1}}, 'first numeric matches');
    return {header, parsed: flow.parseFlowHeaders(header)};
  });

  await check('PC28-flow-parser-malformed-and-falsy-inputs', () => {
    for (const input of ['', null, false, 0, undefined]) assert(flow.parseFlowHeaders(input) === undefined, 'empty flow returns undefined');
    const errors = ['total=2', 'download=2', 'DOWNLOAD=1; TOTAL=2', 1, 'download=1; total=2; app_url=%ZZ'].map(input => errorName(() => flow.parseFlowHeaders(input)));
    equal(errors, ['TypeError','TypeError','TypeError','TypeError','URIError'], 'missing/case-sensitive fields and URI decoding reject');
    assert(errorName(() => flow.getFlowField(null)) === 'TypeError', 'headers must be an object');
    return errors;
  });

  await check('PC29-flow-units-quota-and-expiry-boundary', () => {
    const units = [flow.flowTransfer(1023), flow.flowTransfer(1024), flow.flowTransfer(1536), flow.flowTransfer(-1024), flow.flowTransfer(1024,'YB')];
    equal(units, [{value:'1023',unit:'B'}, {value:'1',unit:'KB'}, {value:'1.5',unit:'KB'}, {value:'-1024',unit:'B'}, {value:'1024',unit:'YB'}], 'binary units and negative values');
    const messages = [undefined, {total:1024,usage:{upload:512,download:512}}, {total:1024,usage:{upload:1024,download:512}}].map(input => {
      try { flow.validCheck(input); } catch (error) { return error.message; }
      throw new Error('quota failure expected');
    });
    equal(messages, ['没有流量信息','流量已用完: 1 KB / 1 KB','流量已用完: 1.5 KB / 1 KB'], 'quota errors');
    assert(flow.validCheck({expires:probeNow/1000,total:0}) === undefined, 'exact expiry boundary and unlimited quota');
    assert(flow.validCheck({total:10,usage:{upload:-10,download:10}}) === undefined, 'negative usage retained');
    return {units,messages};
  });

  await check('PC30-flow-reset-calendar-and-invalid-options', () => {
    // The isolated fixture uses UTC; deployment timezone variants remain separate acceptance cases.
    equal([new Date().getFullYear(),new Date().getMonth(),new Date().getDate()], [2033,4,18], 'shared fixed UTC clock');
    const result = [flow.getRmainingDays(20), flow.getRmainingDays('18'), flow.getRmainingDays({resetDay:1}),
      flow.getRmainingDays({startDate:'2033-05-01',cycleDays:10}), flow.getRmainingDays({startDate:'2033-05-08',cycleDays:'10'})];
    equal(result, [2,31,14,3,0], 'monthly equality rolls forward; cyclic equality returns zero');
    for (const input of [undefined, 0, 32, 'invalid', {startDate:'2033-06-01',cycleDays:10}, {startDate:'invalid',cycleDays:10}, {startDate:'2033-05-01',cycleDays:-1}])
      assert(flow.getRmainingDays(input) === undefined, 'invalid options return undefined');
    assert(typeof flow.getRemainingDays === 'undefined', 'published spelling stays getRmainingDays');
    return result;
  });

  await check('PC31-flow-normalization-duplicates-and-fallback', () => {
    const source = 'download=1.6; upload=-1.5; total=Infinity; expire=0; reset_day=-1; download=9; app_url=https%3A%2F%2Fexample.invalid%2F; plan_name=套餐%F0%9F%99%82; extra=a=b; broken=%ZZ; ignored';
    const text = 'download=2; upload=-2; total=0; expire=; reset_day=; app_url=https%3A%2F%2Fexample.invalid%2F; plan_name=%E5%A5%97%E9%A4%90%F0%9F%99%82; extra=a%3Db; broken=%25ZZ';
    assert(flow.normalizeFlowHeader(source) === text, 'first duplicate, numeric rounding and malformed URI fallback');
    equal(flow.normalizeFlowHeader(source,true), {'subscription-userinfo':'download=2; upload=-2; total=0; expire=; reset_day=; extra=a%3Db; broken=%25ZZ',
      'profile-web-page-url':'https://example.invalid/', 'plan-name':'套餐🙂'}, 'split metadata headers');
    assert(flow.normalizeFlowHeader(null) === null, 'invalid input fallback');
    equal(flow.normalizeFlowHeader(null,true), {'subscription-userinfo':null}, 'split invalid input fallback');
    assert(flow.normalizeFlowHeader('Total=9; total=4; total=5') === 'Total=9; total=4', 'key case retained');
    return text;
  });

  await check('PC32-public-helpers-in-actual-script-operator', async () => {
    const result = await operator(`function operator(proxies) {
      if (flowUtils.getRmainingDays(20) !== 2 || typeof flowUtils.getRemainingDays !== 'undefined') throw new Error('flow injection');
      return proxies.map(proxy => ({...proxy, name: ProxyUtils.getFlag('香港') + '-' + ProxyUtils.hex_md5(proxy.name),
        helper: ProxyUtils.JSON5.parse('{ok:true,zero:0,}'), flow: flowUtils.parseFlowHeaders('download=1; total=2'),
        ip: new ProxyUtils.ipAddress.Address6('2001:db8::1').correctForm()}));
    }`, [{name:'abc',unknown:{flag:false}}]);
    equal(result, [{name:'🇭🇰-900150983cd24fb0d6963f7d28e17f72',unknown:{flag:false},helper:{ok:true,zero:0},
      flow:{total:2,usage:{upload:0,download:1}},ip:'2001:db8::1'}], 'source wrapper exposes real helpers');
    return result;
  });

  const headersCache = host.headersResourceCache;
  const beforeSettings = $substore.read('settings');
  $substore.write({...beforeSettings, defaultFlowUserAgent:'Flow Default/1', headersCacheTtl:1.5}, 'settings');
  const flowValue = 'upload=10; download=20; total=1000; app_url=https%3A%2F%2Fexample.invalid%2Fflow; plan_name=Flow%20plan';
  try {
    await check('PC33-flow-download-short-circuits', async () => {
      const before = JSON.stringify(headersCache.resourceCache);
      for (const url of ['', '/api/file/local', 'file:///tmp/local', probeUpstream + '/host/flow/head#noFlow'])
        assert(await flow.getFlowHeaders(url) === undefined, 'non-HTTP and noFlow return undefined');
      assert(JSON.stringify(headersCache.resourceCache) === before, 'short circuits do not cache');
      return {requests:0, cacheUnchanged:true};
    });

    await check('PC34-flow-head-cache-ttl-and-no-cache', async () => {
      const start = probeNow;
      const url = probeUpstream + '/host/flow/head';
      const customHeaders = {'X-Flow-Token':'direct', 'User-Agent':'Flow Helper/1'};
      const get = suffix => flow.getFlowHeaders(url + suffix, undefined, undefined, undefined, undefined, customHeaders);
      try {
        const id = utils.hex_md5(JSON.stringify({'user-agent':'Flow Helper/1','x-flow-token':'direct'}) + url);
        assert(await get('#headersCacheTtl=2') === flowValue, 'HEAD header result');
        equal(headersCache.resourceCache[id], {time:start+2000,data:flowValue}, 'fragment TTL is seconds');
        assert(await get('') === flowValue, 'cached call makes no request');
        assert(await get('#noCache&headersCacheTtl=2') === flowValue, 'noCache fetches and updates cache');
        probeNow += 2000;
        assert(await get('') === flowValue, 'cache expiry equality is inclusive');
        probeNow += 1;
        assert(await get('') === flowValue, 'expired cache fetches again');
        equal(headersCache.resourceCache[id], {time:probeNow+1500,data:flowValue}, 'settings TTL is seconds');
        return {id, cached:headersCache.resourceCache[id]};
      } finally { probeNow = start; }
    });

    await check('PC35-flow-head-to-get-fallback', async () => {
      const result = await flow.getFlowHeaders(probeUpstream + '/host/flow/get');
      assert(result === flowValue, 'empty HEAD falls back to GET headers');
      return result;
    });

    await check('PC36-flow-url-body-and-encoded-headers', async () => {
      const flowUrl = probeUpstream + '/host/flow/body#' + encodeURIComponent(JSON.stringify({headers:{'X-Flow-Token':'fragment','User-Agent':'Flow Fragment/1'}}));
      const result = await flow.getFlowHeaders('https://unused.invalid/sub', undefined, undefined, undefined, flowUrl);
      assert(result === 'upload=30; download=40; total=2000', 'flowUrl takes precedence and trims body');
      return result;
    });

    await check('PC37-flow-body-fallback-and-status-rejection', async () => {
      const results = [];
      for (const path of ['fallback','badstatus']) results.push(await flow.getFlowHeaders(undefined, 'Flow Explicit/1', undefined, undefined,
        probeUpstream + '/host/flow/' + path, JSON.stringify({'X-Flow-Token':'argument'})));
      equal(results, [flowValue,undefined], 'invalid body can use headers; HTTP failure rejects before saving headers');
      assert(results[1] === undefined, 'HTTP failure returns undefined, not null');
      return {bodyFallback:results[0], statusRejectedUndefined:true};
    });

    await check('PC38-flow-rejection-empty-and-timeout', async () => {
      const count = Object.keys(headersCache.resourceCache).length;
      assert(await flow.getFlowHeaders(undefined, undefined, undefined, undefined, probeUpstream + '/host/flow/rejected') === undefined, 'failed flow URL returns undefined');
      assert(await flow.getFlowHeaders(probeUpstream + '/host/flow/empty') === '', 'empty HEAD and GET return an empty string');
      assert(await flow.getFlowHeaders(undefined, undefined, 30, undefined, probeUpstream + '/host/flow/delay') === undefined, 'flow URL timeout returns undefined');
      await $substore.wait(240);
      assert(Object.keys(headersCache.resourceCache).length === count, 'errors and late timeout response do not cache');
      return {rejectedUndefined:true, empty:'', timeoutUndefined:true, cacheUnchanged:true};
    });

    await check('PC39-flow-request-queue-and-wait-time', async () => {
      $substore.write({...$substore.read('settings'),backendRequestConcurrency:1,backendRequestConcurrencyWaitTime:5}, 'settings');
      const result = await Promise.all(['first','second'].map(name => flow.getFlowHeaders(probeUpstream + '/host/flow/queued/' + name)));
      equal(result, ['download=0; total=1; plan_name=first-active-1','download=0; total=1; plan_name=second-active-1'], 'actual network operations are serialized by the source queue');
      return result;
    });

    await check('PC40-flow-http-injected-into-pending-script', async () => {
      const result = await operator(`async function operator(proxies) {
        const value = await flowUtils.getFlowHeaders($arguments.url);
        return proxies.map(proxy => ({...proxy,flow:flowUtils.parseFlowHeaders(value)}));
      }`, [{name:'flow-script',unknown:false}], {url:probeUpstream+'/host/flow/head'});
      equal(result, [{name:'flow-script',unknown:false,flow:{total:1000,usage:{upload:10,download:20},appUrl:'https://example.invalid/flow',planName:'Flow plan'}}], 'flow helper uses pending Rust HTTP from the actual script wrapper');
      equal(JSON.parse($substore.read('#sub-store-cached-headers-resource')), headersCache.resourceCache, 'persisted header cache');
      return {result, cacheEntries:Object.keys(headersCache.resourceCache).length};
    });
  } finally { $substore.write(beforeSettings, 'settings'); }

  await check('PC125-script-http-compressed-subscription-responses', async () => {
    const results = {};
    for (const mode of ['gzip','x-gzip','deflate','compress','x-compress','nested','status']) {
      const response = await $substore.http.get({url:probeUpstream+'/host/compressed/'+mode});
      assert(response.body === '订阅🙂\r\nsecond\n', 'decoded subscription text: '+mode);
      assert(response.statusCode === (mode === 'status' ? 503 : 200), 'status retained: '+mode);
      assert(response.headers['content-encoding'] === undefined && response.headers['content-length'] === undefined, 'decoded headers removed: '+mode);
      results[mode] = {body:response.body,status:response.statusCode};
    }
    const raw = await $substore.http.get({url:probeUpstream+'/host/compressed/unknown',encoding:null});
    const bytes = Array.from(new Uint8Array(raw.body));
    assert(bytes[0] === 31 && bytes[1] === 139 && Number(raw.headers['content-length']) === bytes.length, 'unknown chain stays raw');
    assert(raw.headers['content-encoding'] === 'gzip, unknown', 'unknown encoding retained');
    let message;
    try { await $substore.http.get({url:probeUpstream+'/host/compressed/too-many'}); } catch (error) { message=error.message; }
    assert(message === 'too many content-encodings in response: 6, maximum allowed is 5', 'source encoding-chain limit');
    return {results,raw:bytes,message};
  });

  await check('PC126-script-http-forward-proxy-and-basic-auth', async () => {
    const results = [];
    for (const proxy of [probeUpstream,probeUpstream.replace('http://','http://fixture:secret@')]) {
      const response = await $substore.http.get({url:'http://ip-api.com/json/1.2.3.4',proxy,headers:{Accept:null}});
      const body = JSON.parse(response.body);
      assert(response.statusCode === 200 && body.query === '1.2.3.4' && body.countryCode === 'US', 'actual HTTP forward proxy');
      results.push(body);
    }
    let message;
    try { await $substore.http.get({url:'http://ip-api.com/json/1.2.3.4',proxy:probeUpstream,headers:{'Proxy-Authorization':'forbidden'}}); }
    catch (error) { message=error.message; }
    assert(message === 'Proxy-Authorization should be sent in ProxyAgent constructor', 'proxy credentials have one owner');
    return {results,message};
  });
};
