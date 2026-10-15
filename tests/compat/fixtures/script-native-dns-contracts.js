// The same assertions call actual Node DNS exports and Rust-owned raw transports.
globalThis.runNativeDnsContracts = async function(check, host, operator, assert, equal) {
  const dns = probeDnsServers;
  const cache = host.resourceCache;
  const ipv4 = ['198.51.100.7', '198.51.100.9'];
  const query = (domain, transport, options = {}) => host.resolveDns({url:dns[transport],domain,timeout:2000,...options});
  const custom = (domain, url, insecure, timeout = 2000) =>
    host.DOMAIN_RESOLVERS.Custom(domain,'IPv4',false,timeout,undefined,url,insecure,1,5000);
  async function rejects(callback, expected) {
    let message;
    try { await callback(); } catch (error) { message = error.message; }
    assert(typeof message === 'string' && message.length > 0, 'native DNS requires a diagnostic');
    if (expected) assert(message === expected, 'native DNS diagnostic: ' + message);
    return message;
  }
  async function certificateRejects(callback) {
    const message = await rejects(callback);
    assert(/cert|self.signed|unknown.?issuer|untrusted/i.test(message), 'certificate failure must identify certificate validation: ' + message);
  }

  await check('PC53-native-dns-address-validation', async () => {
    const inputs = ['127.0.0.1',' ::1 ','[2001:db8::1]:5353','TCP://dns.fixture:5353/','tls://dns.fixture',
      'https://resolver.invalid/dns-query?value=1'];
    const parsed = inputs.map(host.parseDnsResolver);
    equal(parsed, [
      {protocol:'udp',host:'127.0.0.1',port:53}, {protocol:'udp',host:'::1',port:53},
      {protocol:'udp',host:'2001:db8::1',port:5353}, {protocol:'tcp',host:'dns.fixture',port:5353},
      {protocol:'tls',host:'dns.fixture',port:853}, {protocol:'doh',url:inputs[5]},
    ], 'source parser defaults, IPv6, whitespace and schemes');
    for (const value of ['',null,'  ']) await rejects(() => host.parseDnsResolver(value), '自定义 DNS 不能为空');
    await rejects(() => host.parseDnsResolver('ftp://dns.fixture'), '自定义 DNS 不支持 ftp 协议');
    await rejects(() => host.parseDnsResolver('udp://127.0.0.1:0'), '自定义 DNS 端口号应为 1-65535 的整数');
    for (const value of ['udp://user@dns.fixture','tcp://dns.fixture/path','tls://dns.fixture?x=1','udp://dns.fixture#fragment','udp://[broken'])
      await rejects(() => host.parseDnsResolver(value), '自定义 DNS 地址格式无效: ' + value);
    for (const value of ['auto','system','h3://dns.fixture','quic://dns.fixture'])
      await rejects(() => host.resolveDns({url:value,domain:'not-sent.fixture'}), 'auto/system 和 DoH3/DoQ 仅支持 Loon Build ≥ 988');
    return {parsed,invalidRejected:14};
  });

  await check('PC54-udp-ipv4-ipv6-and-edns', async () => {
    const a = await query('udp-a.fixture','udp',{edns:'203.0.113.99'});
    const hostname = await query('udp-hostname.fixture','udpName');
    const aaaa = await query('udp6-aaaa.fixture','udp6',{type:'AAAA',edns:'2001:db8:1234:5678::99'});
    equal(a.answers.filter(record => record.type === 'A').map(record => record.data), ipv4, 'UDP IPv4 answers');
    equal(aaaa.answers.filter(record => record.type === 'AAAA').map(record => record.data), ['2001:db8::7'], 'UDP IPv6 answers');
    assert(hostname.answers.length === 3, 'UDP hostname selects the socket address family');
    return {a,hostname,aaaa};
  });

  await check('PC55-tcp-fragments-inactivity-and-first-frame', async () => {
    const fragmented = await query('fragment-tcp.fixture','tcp',{timeout:200});
    const ipv6 = await query('extra-tcp6.fixture','tcp6',{type:'AAAA'});
    assert(fragmented.answers.length === 3 && ipv6.answers.length === 2, 'split prefix/body and invalid second frame');
    return {fragmented,ipv6};
  });

  await check('PC56-tcp-premature-header-and-body-eof', async () => {
    for (const domain of ['end-header-tcp.fixture','end-body-tcp.fixture'])
      await rejects(() => query(domain,'tcp'), 'DNS TCP connection ended before response');
    return {prematureRejected:2};
  });

  await check('PC57-udp-and-tcp-malformed-packet', async () => {
    for (const transport of ['udp','tcp']) await rejects(() => query('malformed-' + transport + '.fixture',transport));
    return {malformedRejected:2};
  });

  await check('PC58-udp-and-tcp-timeout-no-late-cache', async () => {
    const before = JSON.stringify(cache.resourceCache);
    for (const transport of ['udp','tcp'])
      await rejects(() => custom('timeout-' + transport + '.fixture',dns[transport],false,50), 'DNS ' + transport.toUpperCase() + ' query timeout');
    await $substore.wait(220);
    assert(JSON.stringify(cache.resourceCache) === before, 'late raw DNS cannot populate cache');
    return {timeouts:2,cacheUnchanged:true};
  });

  await check('PC59-dot-default-verification-and-explicit-boolean', async () => {
    // Use the source default budget for the first platform TLS handshake; tight idle tests stay explicit.
    await certificateRejects(() => query('not-sent.fixture','tlsName',{timeout:8000}));
    await certificateRejects(() => query('not-sent.fixture','tlsName',{timeout:8000,skipCertVerify:1}));
    const result = await query('tls-hostname.fixture','tlsName',{timeout:8000,skipCertVerify:true});
    assert(result.answers.length === 3, 'explicit boolean permits fixture certificate');
    return {defaultRejected:true,numberTrueRejected:true,result};
  });

  await check('PC60-dot-ip-without-sni', async () => {
    const result = await query('tls-ip.fixture','tls',{type:'AAAA',skipCertVerify:true});
    equal(result.answers.filter(record => record.type === 'AAAA').map(record => record.data), ['2001:db8::7'], 'DoT IP answers');
    return result;
  });

  await check('PC61-dot-fragments-eof-and-timeout', async () => {
    const result = await query('fragment-tls.fixture','tlsName',{timeout:200,skipCertVerify:true});
    await rejects(() => query('end-body-tls.fixture','tlsName',{skipCertVerify:true}), 'DNS TLS connection ended before response');
    await rejects(() => custom('timeout-tls.fixture',dns.tlsName,true,50), 'DNS TLS query timeout');
    await $substore.wait(220);
    return {result,prematureRejected:true,timeoutRejected:true};
  });

  await check('PC62-custom-mixed-native-fallback-and-cache', async () => {
    const urls = dns.udp + '\n' + dns.tcp;
    const result = await custom('fallback.fixture',urls);
    equal(result, ipv4, 'empty UDP falls back to TCP');
    assert(result._resolverUrl === dns.tcp, 'native winner metadata');
    assert(await custom('fallback.fixture',urls) === result, 'native cache alias hit makes no request');
    return {result,resolverUrl:result._resolverUrl};
  });

  await check('PC63-custom-dot-certificate-cache-partition', async () => {
    const result = await custom('tls-cache.fixture',dns.tlsName,true);
    equal(result, ipv4, 'insecure DoT cache result');
    assert(await custom('tls-cache.fixture',dns.tlsName,true) === result, 'insecure live cache retains alias');
    const before = JSON.stringify(cache.resourceCache);
    await certificateRejects(() => custom('tls-cache.fixture',dns.tlsName,false));
    assert(JSON.stringify(cache.resourceCache) === before, 'verified query cannot use insecure cache or change it on failure');
    return {result,secureRejected:true,cacheUnchanged:true};
  });

  await check('PC64-native-dns-in-async-script', async () => {
    const result = await operator(`async function operator(proxies) {
      if ($substore.env.isNode !== $arguments.node) throw new Error('native DNS spoofed Node');
      if (!$arguments.node && (typeof require !== 'undefined' || typeof process !== 'undefined' || typeof $substore.node !== 'undefined')) throw new Error('native DNS exposed Node');
      const addresses = await DOMAIN_RESOLVERS.Custom('injected-native.fixture','IPv4',false,2000,undefined,$arguments.url);
      return proxies.map(proxy => ({...proxy,addresses}));
    }`, [{name:'native-dns',unknown:false}], {url:dns.tcp,node:probeKind === 'node'});
    equal(result, [{name:'native-dns',unknown:false,addresses:ipv4}], 'pending raw DNS in actual wrapper');
    equal(JSON.parse($substore.read('#sub-store-cached-resource')), cache.resourceCache, 'native DNS cache persists');
    return result;
  });
};
