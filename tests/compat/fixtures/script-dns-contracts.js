// Shared assertions use real module exports in Node and verbatim helpers over Rust HTTP.
globalThis.runDnsContracts = async function(check, host, operator, assert, equal) {
  const doh = host.ProxyUtils.doh;
  const custom = host.DOMAIN_RESOLVERS.Custom;
  const cache = host.resourceCache;
  const url = mode => probeUpstream + '/host/dns/' + mode;
  const ipv4 = ['198.51.100.7', '198.51.100.9'];
  const resolve = (domain, options = {}) => custom(domain, options.type || 'IPv4', options.noCache, options.timeout || 2000,
    options.edns, options.url || url('good'), options.insecure, options.concurrency, options.ttl);
  async function rejects(callback, expected) {
    let message;
    try { await callback(); } catch (error) { message = error.message; }
    assert(typeof message === 'string' && message.length > 0, 'DNS failure must reject with a diagnostic');
    if (expected) assert(message === expected, 'DNS diagnostic: ' + message);
    return message;
  }

  await check('PC41-doh-a-packet-edns-and-cname', async () => {
    const result = await doh({url:url('good'),domain:'packet.fixture',type:'A',edns:'203.0.113.99',timeout:2000});
    assert(result.type === 'response' && result.id === 0 && result.rcode === 'NOERROR', 'DNS response header');
    equal(result.questions, [{name:'packet.fixture',type:'A',class:'IN'}], 'question survives DNS response');
    equal(result.answers.map(record => [record.type,record.data,record.ttl]),
      [['CNAME','alias.fixture',60],['A',ipv4[0],60],['A',ipv4[1],60]], 'mixed DNS records');
    return result;
  });

  await check('PC42-doh-aaaa-and-ipv6-edns', async () => {
    const result = await doh({url:url('good'),domain:'ipv6.fixture',type:'AAAA',edns:'2001:db8:1234:5678::99',timeout:2000});
    equal(result.answers.map(record => [record.type,record.data]), [['CNAME','alias.fixture'],['AAAA','2001:db8::7']], 'IPv6 response');
    return result;
  });

  await check('PC43-doh-defaults-status-and-rcode', async () => {
    const status = await doh({url:url('status'),domain:'status.fixture',timeout:2000});
    const rcode = await doh({url:url('rcode'),domain:'rcode.fixture',timeout:2000});
    assert(status.questions[0].type === 'A' && status.answers.length === 3, 'valid DNS body survives HTTP 503');
    assert(rcode.rcode === 'NXDOMAIN' && rcode.answers.length === 3, 'doh decodes rcode without enforcing success');
    return {status,rcode};
  });

  await check('PC44-doh-malformed-and-custom-timeout', async () => {
    await rejects(() => doh({url:url('malformed'),domain:'malformed.fixture',timeout:2000}));
    const before = JSON.stringify(cache.resourceCache);
    const message = await rejects(() => resolve('timeout.fixture', {url:url('delay'),timeout:30}));
    assert(message.endsWith('exceeds the timeout 30 ms'), 'HTTP timeout reaches DNS caller');
    await $substore.wait(240);
    assert(JSON.stringify(cache.resourceCache) === before, 'late response cannot populate DNS cache');
    return {malformedRejected:true,timeoutRejected:true,cacheUnchanged:true};
  });

  await check('PC45-custom-dns-cache-alias-ttl-and-key', async () => {
    const start = probeNow;
    const normalized = url('good');
    const id = host.ProxyUtils.hex_md5(`CUSTOM:${normalized}:cache.fixture:IPv4`);
    try {
      const first = await resolve('cache.fixture', {url:' \r\n' + normalized + '  \r\n',ttl:50,edns:'203.0.113.99'});
      equal(first, ipv4, 'custom filters CNAME');
      const descriptor = Object.getOwnPropertyDescriptor(first, '_resolverUrl');
      assert(first._resolverUrl === normalized && descriptor.enumerable === false && descriptor.configurable === true && descriptor.writable === false,
        'winning resolver metadata is non-enumerable and read-only');
      assert(cache.resourceCache[id].data.result === first && cache.resourceCache[id].time === start + 50, 'cache retains aliases and millisecond TTL');
      first.push('cached-alias');
      assert(await resolve('cache.fixture', {edns:'198.51.100.2',ttl:1}) === first, 'EDNS and TTL do not partition a live cache key');
      probeNow += 50;
      assert(await resolve('cache.fixture') === first, 'expiry equality is inclusive');
      const fresh = await resolve('cache.fixture', {noCache:true,ttl:50});
      equal(fresh, ipv4, 'noCache refreshes');
      assert(fresh !== first, 'refresh replaces old array');
      probeNow += 51;
      equal(await resolve('cache.fixture', {ttl:50}), ipv4, 'expired DNS cache fetches');
      equal(await resolve('cache.fixture', {insecure:true,ttl:50}), ipv4, 'insecure flag uses a separate cache key even for HTTP');
      const insecureId = host.ProxyUtils.hex_md5(`CUSTOM:INSECURE:${normalized}:cache.fixture:IPv4`);
      assert(cache.resourceCache[insecureId].time === probeNow + 50, 'separate insecure cache record');
      return {data:cache.resourceCache[id].data,time:cache.resourceCache[id].time,descriptor:{enumerable:descriptor.enumerable,configurable:descriptor.configurable,writable:descriptor.writable}};
    } finally { probeNow = start; }
  });

  await check('PC46-custom-dns-aaaa-filter', async () => {
    const result = await resolve('custom6.fixture', {type:'IPv6',edns:'2001:db8:1234:5678::99'});
    equal(result, ['2001:db8::7'], 'only requested address family survives');
    return result;
  });

  await check('PC47-custom-dns-empty-wrong-and-nxdomain', async () => {
    const before = JSON.stringify(cache.resourceCache);
    for (const mode of ['empty','wrong','nxdomain'])
      await rejects(() => resolve(mode + '.fixture', {url:url(mode)}), 'No answers');
    assert(JSON.stringify(cache.resourceCache) === before, 'DNS failures leave full cache unchanged');
    return {failures:3,cacheUnchanged:true};
  });

  await check('PC48-custom-dns-sequential-fallback', async () => {
    const result = await resolve('serial.fixture', {url:[url('serial-empty'),url('serial-good'),url('never')].join('\n'),concurrency:'1'});
    equal(result, ipv4, 'sequential fallback succeeds');
    assert(result._resolverUrl === url('serial-good'), 'winning URL identifies the fallback');
    return {result,resolverUrl:result._resolverUrl};
  });

  await check('PC49-custom-dns-bounded-race-and-winner-cache', async () => {
    const urls = [url('race-slow'),url('race-fast'),url('never')].join('\n');
    const result = await resolve('race.fixture', {url:urls,concurrency:2});
    equal(result, ipv4, 'first successful concurrent DNS wins');
    assert(result._resolverUrl === url('race-fast'), 'fast response wins independently of input ordering');
    await doh({url:url('release'),domain:'release.fixture',timeout:2000});
    await $substore.wait(240);
    const cached = await resolve('race.fixture', {url:urls});
    assert(cached === result && cached._resolverUrl === url('race-fast'), 'late loser does not overwrite the cached winner');
    return {result,resolverUrl:result._resolverUrl};
  });

  await check('PC50-custom-dns-exhaustion-retains-errors', async () => {
    const urls = [url('empty'),url('wrong')];
    const message = await rejects(() => resolve('exhausted.fixture', {url:urls.join('\n'),concurrency:1}),
      urls.map(value => value + ': Error: No answers').join('; '));
    return message;
  });

  await check('PC51-custom-dns-invalid-inputs-before-network', async () => {
    for (const value of [null,'',' \r\n']) await rejects(() => custom('invalid.fixture','IPv4',false,1000,undefined,value), '自定义 DNS 不能为空');
    for (const value of [0,-1,1.5,'invalid',Infinity])
      await rejects(() => resolve('invalid.fixture', {concurrency:value}), '多 DNS 并发数应为大于 0 的整数');
    return {emptyRejected:3,concurrencyRejected:5};
  });

  await check('PC52-doh-and-resolver-in-pending-script', async () => {
    const result = await operator(`async function operator(proxies) {
      if ($substore.env.isNode !== $arguments.node) throw new Error('truthful DNS environment');
      const packet = await ProxyUtils.doh({url:$arguments.url,domain:'injected.fixture',timeout:2000});
      const addresses = await DOMAIN_RESOLVERS.Custom('injected-cache.fixture','IPv4',false,2000,undefined,$arguments.url);
      return proxies.map(proxy => ({...proxy,rcode:packet.rcode,addresses}));
    }`, [{name:'dns-script',unknown:false}], {url:url('good'),node:probeKind === 'node'});
    equal(result, [{name:'dns-script',unknown:false,rcode:'NOERROR',addresses:ipv4}], 'actual script wrapper injects DNS helpers over pending HTTP');
    equal(JSON.parse($substore.read('#sub-store-cached-resource')), cache.resourceCache, 'full DNS resource cache persists');
    return result;
  });
};
