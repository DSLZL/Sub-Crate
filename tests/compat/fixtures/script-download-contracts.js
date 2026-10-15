(async () => {
  const host = processorHost, {download, downloadFile, hex_md5} = host.ProxyUtils;
  const observations = {}, passed = [];
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const equal = (a, b, message) => assert(JSON.stringify(a) === JSON.stringify(b), message + ': ' + JSON.stringify(a));
  const check = async (id, callback) => {
    probeDownloadProgress(id, 'start'); observations[id] = await callback();
    passed.push(id); probeDownloadProgress(id, 'end');
  };
  const url = path => probeUpstream + path;
  const file = name => probeMmdbRoot + '/' + name;
  const text = '下载🙂\r\n\0尾\n';
  const settings = value => $substore.write(value, 'settings');
  const reset = () => { settings({}); host.resourceCache.revokeAll(); };
  const fetch = (path, options = {}, flags = {}) => download(url(path), flags.ua, flags.timeout, undefined,
    flags.skip, flags.await, flags.noCache, false, options);
  const id = (path, ua = 'clash.meta/v1.19.23') => hex_md5(ua + url(path));
  const read = name => probeReadFile(file(name));
  async function rejects(callback, message) {
    let caught = false;
    try { await callback(); } catch (error) {
      caught = error instanceof Error;
      if (message) assert(String(error).includes(message), 'unexpected error: ' + error);
    }
    assert(caught, 'must reject with native Error'); return true;
  }
  await check('DL01-defaults-unicode-raw-and-regular-cache', async () => {
    $substore.delete('settings');
    equal(await fetch('/text/default'), text, 'default text');
    equal(await fetch('/text/default', {returnRaw: true}), {result: text, raw: text}, 'cached raw result');
    return {text, cached: host.resourceCache.get(id('/text/default'))};
  });
  await check('DL02-settings-headers-and-fragment-arguments', async () => {
    settings({defaultUserAgent: 'download-default', defaultTimeout: 1500});
    equal(await fetch('/text/settings'), text, 'settings');
    const ascii = JSON.stringify({'User-Agent': 'download-custom', 'X-Download-Token': 'token-2'});
    equal(await fetch('/text/headers#' + encodeURIComponent(JSON.stringify({headers: ascii}))), text, 'JSON fragment');
    equal(await fetch('/text/pairs#headers=' + encodeURIComponent(ascii) + '&noCache=true#noFlow'), text, 'pair fragment');
    equal(await fetch('/text/invalid-headers#headers=%7Bbroken'), text, 'invalid headers fallback');
    settings({}); return {settings: true, custom: true, invalidFallback: true};
  });
  await check('DL03-github-proxy-pattern-and-invalid-regex', async () => {
    settings({githubProxy: url('/proxy'), githubProxyRegex: '/text/proxied$'});
    equal(await fetch('/text/proxied'), text, 'proxy prefix');
    equal(await download(url('/proxy') + '/' + url('/text/proxied')), text, 'do not prefix twice');
    settings({githubProxy: url('/proxy'), githubProxyRegex: '['});
    equal(await fetch('/text/invalid-regex'), text, 'invalid regex continues');
    settings({}); return {prefix: true, noDoublePrefix: true, invalidRegex: true};
  });
  await check('DL04-native-local-utf8-empty-and-read-errors', async () => {
    equal(await download(file('local🙂.txt') + '#noFlow'), text, 'native absolute path');
    equal(await download(file('local🙂.txt'), undefined, undefined, undefined, undefined, undefined, undefined, false,
      {returnRaw: true, ageSecretKey: 'ignored-on-local'}), {result: text, raw: text}, 'local raw bypass');
    equal(await download(file('empty.txt')), '', 'empty local is allowed');
    equal(await download(file('invalid-utf8.txt')), 'a�(�(z', 'Node replacement decoding');
    await rejects(() => download(file('missing.txt')), '无法从该路径读取文本内容');
    return {text, empty: '', malformed: 'a�(�(z', missing: true};
  });
  await check('DL05-internal-modules-typed-content-and-missing-sources', async () => {
    $substore.write([{name: '模块🙂', content: text}, {name: 'zero', content: 0}, {name: 'false', content: false}], 'modules');
    $substore.write([], 'files');
    equal(await download('/api/module/' + encodeURIComponent('模块🙂')), text, 'module name decoding');
    equal(await download('/api/module/zero', undefined, undefined, undefined, undefined, undefined, undefined, false,
      {returnRaw: true}), {result: 0, raw: 0}, 'numeric module');
    equal(await download('/api/module/false'), false, 'boolean module');
    await rejects(() => download('/api/module/missing'), '无法加载 module');
    await rejects(() => download('/api/file/missing'), '无法加载 file');
    return {text, zero: 0, flag: false, missing: ['module', 'file']};
  });
  await check('DL06-cache-ttl-inclusive-expiry-and-bypass', async () => {
    reset(); settings({resourceCacheTtl: 2});
    const start = probeNow;
    await fetch('/text/ttl#cacheTtl=1');
    equal(host.resourceCache.gettime(id('/text/ttl')), start + 1000, 'fragment seconds');
    probeNow = start + 1000; await fetch('/text/ttl');
    probeNow++; await fetch('/text/ttl');
    equal(host.resourceCache.gettime(id('/text/ttl')), probeNow + 2000, 'settings seconds');
    await fetch('/text/ttl', {}, {noCache: true});
    await fetch('/text/ttl#noCache');
    probeNow = start; settings({}); return {inclusive: true, expired: true, bypasses: 2};
  });
  await check('DL07-cache-threshold-and-custom-cache-write-eligibility', async () => {
    reset(); settings({cacheThreshold: 0.001});
    await fetch('/text/threshold#cacheKey=oversize'); await fetch('/text/threshold');
    equal(host.resourceCache.get(id('/text/threshold')), null, 'oversize uncached');
    assert($substore.read('#sub-store-cached-custom-oversize') === undefined, 'custom oversize uncached');
    settings({}); return {regular: null, customAbsent: true};
  });
  await check('DL08-optimistic-cache-awaited-refresh-and-error-fallback', async () => {
    reset(); $substore.write('旧缓存🙂', '#sub-store-cached-custom-optimistic');
    const path = '/text/optimistic#cacheKey=optimistic';
    equal(await fetch(path, {}, {await: true}), '旧缓存🙂', 'returns original cached value after awaited refresh');
    equal($substore.read('#sub-store-cached-custom-optimistic'), text, 'refresh persists');
    equal(await fetch(path), text, 'regular optimistic cache');
    $substore.write('回退🙂', '#sub-store-cached-custom-fallback');
    equal(await fetch('/status/503#cacheKey=fallback', {}, {skip: true}), '回退🙂', 'failure fallback');
    return {old: '旧缓存🙂', updated: text, fallback: '回退🙂'};
  });
  await check('DL09-flow-header-cache-ttl-and-validation-bypass', async () => {
    reset();
    await fetch('/text/flow#headersCacheTtl=2&validCheck&noFlow');
    equal(host.headersResourceCache.get(id('/text/flow')), 'upload=1; download=2; total=100; expire=4102444800', 'flow header');
    equal(host.headersResourceCache.gettime(id('/text/flow')), probeNow + 2000, 'headers TTL seconds');
    await fetch('/text/flow-options#validCheck', {noFlow: true});
    equal(await fetch('/text/flow-valid#validCheck'), text, 'valid flow uses downloaded headers');
    await rejects(() => fetch('/text/flow-exhausted#validCheck'));
    equal(host.resourceCache.get(id('/text/flow-exhausted')), text, 'legacy cache write precedes validation');
    equal(await fetch('/text/flow-exhausted#validCheck&noFlow'), text, 'explicit noFlow bypasses exhausted cached flow');
    return {header: host.headersResourceCache.get(id('/text/flow')), ttl: 2000, bypass: true, validation: true, exhaustedCached: true};
  });
  await check('DL10-status-empty-timeout-and-queue-recovery', async () => {
    reset();
    await rejects(() => fetch('/status/404'), 'statusCode: 404');
    await rejects(() => fetch('/text/white'), '远程资源内容为空');
    await rejects(() => fetch('/text/slow', {}, {timeout: 20}), 'exceeds the timeout 20 ms');
    await $substore.wait(160);
    equal(host.resourceCache.get(id('/text/slow')), null, 'late response not cached');
    equal(await fetch('/text/recovered'), text, 'queue recovers');
    return {status: 404, empty: true, timeout: true, recovered: text};
  });
  await check('DL11-both-age-types-raw-cache-key-precedence-and-redaction', async () => {
    reset(); const results = [];
    for (const key of probeDownloadInput.keys) {
      const path = '/age/' + key.type;
      const fragment = '#age-secret-key=' + key['age-secret-key'] + '&cacheKey=age-' + key.type;
      equal(await fetch(path + fragment, {returnRaw: true}), {result: text, raw: text}, 'decrypted raw result');
      equal(host.resourceCache.get(id(path)), key.armor, 'regular cache retains ciphertext');
      equal($substore.read('#sub-store-cached-custom-age-' + key.type), key.armor, 'custom cache retains ciphertext');
      equal(await fetch(path), key.armor, 'unkeyed cached armor');
      equal(await fetch(path + '#age-secret-key=wrong', {ageSecretKey: key['age-secret-key']}), text, 'explicit key precedence');
      let invalidKey;
      try { await fetch(path, {ageSecretKey: 'invalid'}); } catch (error) { invalidKey = error; }
      assert(invalidKey?.type === 'RequestInvalidError' && invalidKey.code === 'INVALID_AGE_SECRET_KEY' && !(invalidKey instanceof Error), 'cached decryption retains the source structured error');
      equal(host.resourceCache.get(id(path)), key.armor, 'failed cache decryption retains armor');
      await rejects(() => fetch('/status/500#age-secret-key=' + key['age-secret-key']), 'statusCode: 500');
      assert(processorDiagnostics.every(line => !line.includes(key['age-secret-key'])), 'secret must be redacted');
      results.push({type: key.type, text, cacheIsArmor: true, redacted: true, invalidKey: {type: invalidKey.type, code: invalidKey.code, message: invalidKey.message}});
    }
    return results;
  });
  await check('DL12-public-helpers-in-async-script-and-persisted-values', async () => {
    const factory = host.factories['Script Operator'];
    const output = await host.ApplyProcessor(factory(`async function operator(proxies) {
      if ($substore.env.isNode !== $arguments.node) throw new Error('environment mismatch');
      if (!$arguments.node && (typeof require !== 'undefined' || typeof process !== 'undefined' || typeof $substore.node !== 'undefined')) throw new Error('Node capability leaked');
      const text = await ProxyUtils.download($arguments.url);
      const file = await ProxyUtils.downloadFile($arguments.binary, $arguments.file);
      $substore.write({text, zero: 0, flag: false}, 'downloadHelper');
      return proxies.map(proxy => ({...proxy, name: text, saved: file === $arguments.file}));
    }`, 'ClashMeta', {node: probeKind === 'node', url: url('/text/reload'), binary: url('/file/small'), file: file('script🙂.bin')},
      {name: 'download'}, {}, {}), [{name: 'before', type: 'ss'}]);
    equal(output, [{name: text, type: 'ss', saved: true}], 'actual script injection');
    return {output, stored: $substore.read('downloadHelper')};
  });
  await check('DL13-binary-status-redirect-limits-and-existing-file-retention', async () => {
    const target = file('binary🙂.bin');
    equal(await downloadFile(url('/file/small'), target), target, 'returns input path');
    equal(Array.from(read('binary🙂.bin')), [0, 255, 128, 13, 10, 65], 'raw binary');
    await downloadFile(url('/redirect/2'), target);
    for (const path of ['/status/201', '/status/404', '/redirect/3', '/loop']) {
      await rejects(() => downloadFile(url(path), target), path.startsWith('/status/') ? 'Failed to download file from ' + url(path) : undefined);
      equal(Array.from(read('binary🙂.bin')), [0, 255, 128, 13, 10, 65], 'error retains existing file');
    }
    return {bytes: [0, 255, 128, 13, 10, 65], redirects: 2, rejected: 4};
  });
  await check('DL14-binary-stream-visible-before-finish-and-partial-on-error', async () => {
    const result = [];
    for (const mode of ['complete', 'broken']) {
      const name = mode + '.bin';
      const pending = downloadFile(url('/stream/' + mode), file(name)).then(() => ({ok: true}), error => ({ok: false, native: error instanceof Error}));
      let prefix;
      for (let i = 0; i < 200; i++) {
        try { prefix = read(name); } catch (_) {}
        if (prefix?.length === 16384) break;
        await $substore.wait(10);
      }
      assert(prefix?.length === 16384 && prefix.every((value, index) => value === index % 256), 'file prefix must be visible before server finishes');
      await $substore.http.post({url: url('/release/' + mode)});
      const response = await pending;
      equal(response, mode === 'complete' ? {ok: true} : {ok: false, native: true}, 'stream outcome');
      const bytes = read(name), expected = mode === 'complete' ? 262147 : 16384;
      equal(bytes.length, expected, 'stream file length');
      assert(bytes.every((value, index) => value === index % 256), 'exact streamed binary');
      result.push({mode, beforeFinish: 16384, length: bytes.length});
    }
    return result;
  });
  await check('DL15-binary-filesystem-errors-and-recovery', async () => {
    await rejects(() => downloadFile(url('/file/small'), file('missing-parent/out.bin')));
    await rejects(() => downloadFile(url('/file/small'), probeMmdbRoot));
    equal(await downloadFile(url('/file/empty'), file('zero.bin')), file('zero.bin'), 'empty binary allowed');
    equal(read('zero.bin').length, 0, 'empty binary file');
    await downloadFile(url('/file/small'), file('recovery.bin'));
    return {missingParent: true, directory: true, emptyLength: 0, recovered: true};
  });
  settings({});
  return {passed, observations, storage: {cache: $substore.cache, root: $substore.root},
    resourceCache: host.resourceCache.resourceCache, headersCache: host.headersResourceCache.resourceCache};
})()
