async function operator(proxies) {
  const OriginalDate = Date;
  let probeNow = 3000000000000;
  globalThis.Date = class extends OriginalDate {
    constructor(...args) { super(...(args.length ? args : [probeNow])); }
    static now() { return probeNow; }
  };
  const savedSettings = $substore.read('settings');
  const savedModules = $substore.read('modules');
  const passed = [], observations = {};
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const equal = (actual, expected, message) => assert(JSON.stringify(actual) === JSON.stringify(expected), message);
  const check = async (id, callback) => { observations[id]=await callback(); passed.push(id); };
  const base = '$UPSTREAM/host/download';
  const text = '下载🙂\r\n\0尾\n';
  const url = path => base + path;
  const id = (path, agent='clash.meta/v1.19.23') => ProxyUtils.hex_md5(agent+url(path));
  const entry = (key, root='resource') => JSON.parse($substore.read('#sub-store-cached-'+root)||'{}')[key];
  const settings = value => $substore.write(value,'settings');
  const fetch = (path, options={}, flags={}) => ProxyUtils.download(url(path),flags.ua,flags.timeout,undefined,
    flags.skip,flags.await,flags.noCache,flags.preprocess,options);
  const rejects = async (callback, message) => {
    let caught=false;
    try { await callback(); } catch(error) {
      caught=error instanceof Error;
      assert(String(error).includes(message),'download error: '+error);
    }
    assert(caught,'download rejects with Error'); return true;
  };
  try {
    settings({});
    await check('DL01-native-defaults-raw-and-durable-cache', async () => {
      equal(await fetch('/text/default'),text,'default text');
      equal(await fetch('/text/default',{returnRaw:true}),{result:text,raw:text},'cache raw result');
      equal(entry(id('/text/default')),{time:probeNow+3600000,data:text},'regular cache stores original text');
      return {text,cached:entry(id('/text/default'))};
    });
    await check('DL02-native-defaults-and-fragment-headers', async () => {
      settings({defaultUserAgent:'download-default',defaultTimeout:1500});
      equal(await fetch('/text/settings'),text,'settings');
      const headers=JSON.stringify({'User-Agent':'download-custom','X-Download-Token':'token-2'});
      equal(await fetch('/text/headers#'+encodeURIComponent(JSON.stringify({headers}))),text,'JSON fragment');
      equal(await fetch('/text/pairs#headers='+encodeURIComponent(headers)+'&noCache=true#noFlow'),text,'pair fragment');
      equal(await fetch('/text/invalid-headers#headers=%7Bbroken'),text,'invalid headers fallback');
      settings({}); return {settings:true,custom:true,invalidFallback:true};
    });
    await check('DL03-native-github-prefix-and-invalid-regex', async () => {
      settings({githubProxy:url('/proxy'),githubProxyRegex:'/text/proxied$'});
      equal(await fetch('/text/proxied'),text,'prefix');
      equal(await ProxyUtils.download(url('/proxy')+'/'+url('/text/proxied')),text,'no double prefix and cache identity');
      settings({githubProxy:url('/proxy'),githubProxyRegex:'['});
      equal(await fetch('/text/invalid-regex'),text,'invalid regex continues');
      settings({}); return {prefix:true,noDoublePrefix:true,invalidRegex:true};
    });
    await check('DL05-native-internal-modules-and-missing-file', async () => {
      $substore.write([{name:'模块🙂',content:text},{name:'zero',content:0},{name:'false',content:false}],'modules');
      equal(await ProxyUtils.download('/api/module/'+encodeURIComponent('模块🙂')),text,'module name');
      equal(await ProxyUtils.download('/api/module/zero',undefined,undefined,undefined,undefined,undefined,undefined,false,
        {returnRaw:true}),{result:0,raw:0},'numeric module');
      equal(await ProxyUtils.download('/api/module/false'),false,'boolean module');
      await rejects(()=>ProxyUtils.download('/api/module/missing'),'无法加载 module');
      await rejects(()=>ProxyUtils.download('/api/file/missing'),'无法加载 file');
      return {text,zero:0,flag:false,missing:['module','file']};
    });
    await check('DL06-native-cache-expiry-equality-and-bypass', async () => {
      settings({resourceCacheTtl:2});
      const start=probeNow;
      try {
        await fetch('/text/ttl#cacheTtl=1');
        assert(entry(id('/text/ttl')).time===start+1000,'fragment seconds');
        probeNow=start+1000; await fetch('/text/ttl');
        probeNow++; await fetch('/text/ttl');
        assert(entry(id('/text/ttl')).time===probeNow+2000,'settings seconds after expiry');
        await fetch('/text/ttl',{}, {noCache:true});
        await fetch('/text/ttl#noCache');
        return {inclusive:true,expired:true,bypasses:2,cached:entry(id('/text/ttl'))};
      } finally { probeNow=start; settings({}); }
    });
    await check('DL07-native-cache-size-and-custom-eligibility', async () => {
      settings({cacheThreshold:0.001});
      await fetch('/text/threshold#cacheKey=native-oversize');
      await fetch('/text/threshold');
      assert(entry(id('/text/threshold'))===undefined,'oversize uncached');
      assert($substore.read('#sub-store-cached-custom-native-oversize')===undefined,'custom oversize uncached');
      settings({}); return {regularAbsent:true,customAbsent:true};
    });
    await check('DL08-native-awaited-optimistic-refresh-and-fallback', async () => {
      $substore.write('旧缓存🙂','#sub-store-cached-custom-native-optimistic');
      equal(await fetch('/text/optimistic#cacheKey=native-optimistic',{}, {await:true}),'旧缓存🙂','returns original optimistic value');
      equal($substore.read('#sub-store-cached-custom-native-optimistic'),text,'awaited refresh persisted');
      equal(await fetch('/text/optimistic#cacheKey=native-optimistic'),text,'regular cache');
      $substore.write('回退🙂','#sub-store-cached-custom-native-fallback');
      equal(await fetch('/status/503#cacheKey=native-fallback',{}, {skip:true}),'回退🙂','provider failure falls back');
      return {old:'旧缓存🙂',updated:text,fallback:'回退🙂'};
    });
    await check('DL09-native-flow-ttl-and-cache-before-validation', async () => {
      await fetch('/text/flow#headersCacheTtl=2&validCheck&noFlow');
      equal(entry(id('/text/flow'),'headers-resource'),{time:probeNow+2000,data:'upload=1; download=2; total=100; expire=4102444800'},'flow TTL');
      equal(await fetch('/text/flow-options#validCheck',{noFlow:true}),text,'options bypass');
      equal(await fetch('/text/flow-valid#validCheck'),text,'valid flow reuses headers');
      await rejects(()=>fetch('/text/flow-exhausted#validCheck'),'流量已用完');
      equal(entry(id('/text/flow-exhausted')).data,text,'cache write precedes quota validation');
      equal(await fetch('/text/flow-exhausted#validCheck&noFlow'),text,'fragment bypasses exhausted cached flow');
      return {header:entry(id('/text/flow'),'headers-resource'),bypass:true,validation:true,exhaustedCached:true};
    });
    await check('DL10-native-status-empty-timeout-and-recovery', async () => {
      await rejects(()=>fetch('/status/404'),'statusCode: 404');
      await rejects(()=>fetch('/text/white'),'远程资源内容为空');
      await rejects(()=>fetch('/text/slow',{}, {timeout:20}),'exceeds the timeout 20 ms');
      await $substore.wait(240);
      assert(entry(id('/text/slow'))===undefined,'late response not cached');
      equal(await fetch('/text/recovered'),text,'queue recovers');
      return {status:404,empty:true,timeout:true,recovered:text};
    });
    await check('DL16-native-preprocess-return-raw-and-invalid-cache', async () => {
      const raw='proxies:\n  - {name: 预处理, type: ss, server: example.invalid, port: 8388, cipher: aes-128-gcm, password: pass}\n';
      const processed='proxies:\n  - {"name":"预处理","type":"ss","server":"example.invalid","port":8388,"cipher":"aes-128-gcm","password":"pass"}\n';
      equal(await fetch('/text/preprocess-good',{returnRaw:true},{preprocess:true}),{result:processed,raw},'preprocessed result and original raw');
      equal(entry(id('/text/preprocess-good')).data,raw,'cache is original body');
      equal(await fetch('/text/preprocess-good'),raw,'without preprocessing');
      for(let i=0;i<2;i++) equal(await fetch('/text/preprocess-invalid',{}, {preprocess:true}),'<!DOCTYPE html>no nodes','invalid resource still returned');
      assert(entry(id('/text/preprocess-invalid'))===undefined,'invalid resource not cached');
      return {result:processed,raw,invalidNotCached:true};
    });
    proxies[0].downloadsVerified=passed;
    proxies[0].downloadResults=observations;
    return proxies;
  } finally {
    $substore.write(savedSettings,'settings');
    $substore.write(savedModules,'modules');
    globalThis.Date=OriginalDate;
  }
}
