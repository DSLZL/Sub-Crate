globalThis.runArtifactContracts = async function(check, host, operator, assert, equal) {
  const utils = host.ProxyUtils;
  const produce = host.produceArtifact;
  const uri = name => 'ss://YWVzLTEyOC1nY206c2VjcmV0@ss.example.com:8388#' + encodeURIComponent(name);
  const names = proxies => proxies.map(proxy => proxy.name);
  const inline = (content, args = {}) => ({type: 'Script Operator', args: {content, arguments: args}});
  const link = content => ({type: 'Script Operator', args: {mode: 'link', content}});
  const local = (name, content = uri(name)) => ({name, displayName: name + '🙂', source: 'local', content});
  const remote = (name, path, extra = {}) => ({name, url: probeUpstream + '/host/artifact/' + path, ...extra});
  async function rejects(callback, part, kind = Error) {
    try { await callback(); } catch (error) {
      assert(error instanceof kind && error.message.includes(part), 'specific artifact error: ' + error);
      return {name: error.name, matched: part};
    }
    throw new Error('Expected artifact rejection: ' + part);
  }
  const a = local('artifact-a'), b = {...local('artifact-b'), tag: ['artifact-tag']};
  const c = local('artifact-c');
  $substore.write([a, b, c], 'subs');
  $substore.write([{name: 'artifact-col', displayName: '组合🙂', subscriptions: [c.name, a.name], subscriptionTags: ['artifact-tag']}], 'collections');
  $substore.write([{name: 'artifact-module🙂', content: 'async function operator(proxies) { await $substore.wait(1); for (const p of proxies) p.name += String($arguments.suffix); return proxies; }'}], 'modules');
  $substore.write([{name: 'artifact-script', source: 'local', content: 'function operator(proxies) { return proxies.map(p => ({...p, name: p.name + "-file"})); }'}], 'files');
  $substore.write([{name: 'artifact-rule', urls: []}], 'rules');
  // Capture policy callbacks only; real notification transports remain a service integration gap.
  const savedNotify = $substore.notify;
  const notifications = [];
  $substore.notify = (...args) => notifications.push(args);
  try {
    await check('PC105-artifact-full-registry-and-pipeline', async () => {
      const registry = ['Useless Filter', 'Region Filter', 'Regex Filter', 'Type Filter', 'Script Filter', 'Conditional Filter',
        'Quick Setting Operator', 'Flag Operator', 'Sort Operator', 'Regex Sort Operator', 'Regex Rename Operator',
        'Regex Delete Operator', 'Script Operator', 'Add Proxies From Subscription Operator', 'Response Transformer',
        'Handle Duplicate Operator', 'Resolve Domain Operator'].sort();
      equal(Object.keys(host.factories).sort(), registry, 'complete source registry');
      const input = [{name: 'b', type: 'ss'}, {name: 'a', type: 'ss'}, {name: 'drop', type: 'vmess'}];
      const result = await utils.process(input, [{type: 'Type Filter', args: ['ss']}, {type: 'Sort Operator', args: 'asc'},
        {type: 'Regex Rename Operator', args: [{expr: '^', now: 'kept-'}]}, {type: 'unknown'},
        {type: 'Sort Operator', args: 'desc', disabled: true}, {type: 'Response Transformer', args: {content: 'throw new Error("wrong stage")'}}]);
      equal(names(result), ['kept-a', 'kept-b'], 'actual non-script pipeline');
      equal(names(input), ['b', 'a', 'drop'], 'source cloning');
      return {registry, result};
    });
    await check('PC106-artifact-local-subscription-and-raw', async () => {
      const normal = JSON.parse(await produce({type: 'sub', name: a.name}));
      const internal = await produce({type: 'subscription', subscription: a, produceType: 'internal'});
      equal(names(normal), [a.name], 'default JSON');
      equal(names(internal), [a.name], 'inline data alias');
      const raw = await produce({type: 'sub', name: a.name, produceType: 'raw', includeRaw: true});
      equal(raw, {result: JSON.stringify([a.content]), raw: [a.content]}, 'raw envelope before parsing');
      return {normal, internal, raw};
    });
    await check('PC107-artifact-script-source-options-and-raw', async () => {
      const subscription = {...a, process: [inline(`function operator(proxies, platform, context) {
        if (platform !== 'JSON' || context.source['artifact-a'].name !== 'artifact-a' || context.raw[0] !== $options.raw || $arguments.zero !== 0) throw new Error('lost artifact context');
        return proxies.map(p => ({...p, name: p.name + '-script', marker: $options.flag, raw: context.raw}));
      }`, {zero: 0})]};
      const result = await produce({type: 'sub', subscription, produceType: 'internal', $options: {raw: a.content, flag: false}});
      equal(names(result), ['artifact-a-script'], 'real artifact script');
      equal([result[0].marker, result[0].raw], [false, [a.content]], 'false and source raw');
      return result;
    });
    await check('PC108-artifact-context-control-and-response-pipeline', async () => {
      const result = await utils.process([{name: 'base'}], [inline('function operator(p, t, context) { context.process = {type: "enable", customNames: ["run"]}; return p; }'),
        {...inline('throw new Error("disabled by context")'), customName: 'skip'},
        {...inline('function operator(p) { p[0].name += "-run"; return p; }'), customName: 'run'}]);
      equal(names(result), ['base-run'], 'shared mutable pipeline context');
      const response = await utils.processResponse({statusCode: 201, header: {before: 'yes'}, body: false}, [
        {type: 'Script Operator', args: {content: 'throw new Error("wrong stage")'}},
        {type: 'Response Transformer', args: {content: 'function transformFunction(response) { response.headers.after = "yes"; response.body = 0; return response; }'}},
      ]);
      equal([response.status, response.body, response.headers, response.header === response.headers], [201, 0, {before: 'yes', after: 'yes'}, true], 'response aliases and zero');
      return {result, response};
    });
    await check('PC109-artifact-internal-script-links-and-arguments', async () => {
      const input = [{name: 'base'}];
      const first = await utils.process(input, [link('/api/module/' + encodeURIComponent('artifact-module🙂') + '#' + encodeURIComponent(JSON.stringify({suffix: 0})))]);
      const second = await utils.process(input, [link('/api/module/' + encodeURIComponent('artifact-module🙂') + '#suffix=false')]);
      const file = await utils.process(input, [link('/api/file/artifact-script')]);
      equal([names(first), names(second), names(file)], [['base0'], ['basefalse'], ['base-file']], 'real module and generated file script loading');
      return {first, second, file};
    });
    await check('PC110-artifact-remote-script-loading', async () => {
      const result = await utils.process([{name: 'base'}], [link(probeUpstream + '/host/artifact/script#suffix=remote')]);
      equal(names(result), ['base-remote'], 'remote script and fragment arguments');
      return result;
    });
    await check('PC111-artifact-subscription-download-preprocess', async () => {
      const subscription = remote('artifact-clash', 'clash');
      const first = await produce({type: 'sub', subscription, includeRaw: true});
      const second = await produce({type: 'sub', subscription, includeRaw: true});
      equal(first, second, 'second download uses actual source cache');
      equal(names(JSON.parse(first.result)), ['remote🙂'], 'Clash preprocessing and parsing');
      assert(first.raw[0].startsWith('proxies:\n') && first.raw[0].includes('port: 8388'), 'unprocessed source retained');
      const key = utils.hex_md5('clash.meta/v1.19.23' + subscription.url);
      assert(host.resourceCache.get(key).startsWith('proxies:\n'), 'cache retains original raw body');
      return {first, cached: host.resourceCache.get(key)};
    });
    await check('PC112-artifact-invalid-subscription-cache-eligibility', async () => {
      const url = probeUpstream + '/host/artifact/invalid';
      const result = [];
      for (let i = 0; i < 2; i++) result.push(await utils.download(url, undefined, undefined, undefined, undefined, undefined, undefined, true, {returnRaw: true}));
      equal(result, [{result: 'not a subscription', raw: 'not a subscription'}, {result: 'not a subscription', raw: 'not a subscription'}], 'invalid input returned but never cached');
      equal(host.resourceCache.get(utils.hex_md5('clash.meta/v1.19.23' + url)), null, 'ineligible source absent');
      return result;
    });
    await check('PC113-artifact-subscription-merge-and-overrides', async () => {
      const result = [];
      for (const mergeSources of ['localFirst', 'remoteFirst']) {
        const item = await produce({type: 'sub', subscription: a, url: probeUpstream + '/host/artifact/sub', content: uri('override'),
          mergeSources, ua: 'Artifact Override/1', noCache: true, includeRaw: true});
        equal(names(JSON.parse(item.result)), mergeSources === 'localFirst' ? ['override', 'remote'] : ['remote', 'override'], 'explicit merge order');
        result.push(item);
      }
      return result;
    });
    await check('PC114-artifact-collection-order-tags-and-context', async () => {
      const collection = $substore.read('collections')[0];
      collection.process = [inline('function operator(proxies, platform, context) { if (context.source._collection.name !== "artifact-col" || Object.keys(context.raw).length !== 3 || $options.flag !== false) throw new Error("collection context"); return proxies; }')];
      const result = await produce({type: 'collection', name: collection.name, produceType: 'internal', includeRaw: true, $options: {flag: false}});
      equal(names(result.result), [c.name, a.name, b.name], 'explicit order then nonduplicate tagged subscriptions');
      assert(result.result.every(p => p._collectionName === collection.name && p._collectionDisplayName === '组合🙂'), 'collection provenance');
      equal(result.raw, {[c.name]: [c.content], [a.name]: [a.content], [b.name]: [b.content]}, 'raw by source name');
      return result;
    });
    await check('PC115-artifact-collection-remote-ua-and-order', async () => {
      $substore.write([...$substore.read('subs'), remote('artifact-slow', 'slow', {passThroughUA: true, noCache: true}), remote('artifact-fast', 'sub', {ua: 'Artifact Fixed/1', noCache: true})], 'subs');
      $substore.write([...$substore.read('collections'), {name: 'artifact-remote-col', subscriptions: ['artifact-slow', 'artifact-fast']}], 'collections');
      const result = await produce({type: 'col', name: 'artifact-remote-col', ua: 'Artifact Pass/1', noCache: true, includeRaw: true});
      equal(names(JSON.parse(result.result)), ['slow', 'remote'], 'completion order does not change source order');
      return result;
    });
    await check('PC116-artifact-subscription-fallback-and-notification', async () => {
      const before = notifications.length;
      const bad = local('artifact-empty', 'invalid://empty');
      const rejected = await rejects(() => produce({type: 'sub', subscription: bad}), '不含有效节点');
      const quiet = await produce({type: 'sub', subscription: {...bad, ignoreFailedRemoteSub: 'fallbackQuiet'}, includeRaw: true});
      equal(quiet, {result: '[]', raw: [bad.content]}, 'fallback preserves source raw');
      equal(notifications.length, before, 'quiet fallback');
      equal(await produce({type: 'sub', subscription: {...bad, ignoreFailedRemoteSub: 'fallbackNotify'}}), '[]', 'notifying fallback');
      equal(notifications.length, before + 1, 'one fallback notification');
      await produce({type: 'sub', subscription: local('artifact-duplicates', uri('same') + '\n' + uri('same'))});
      equal(notifications.length, before + 2, 'duplicate notification');
      return {rejected, quiet, notifications: notifications.slice(before)};
    });
    await check('PC117-artifact-collection-failure-policy', async () => {
      $substore.write([...$substore.read('subs'), local('artifact-bad')], 'subs');
      $substore.write([...$substore.read('collections'), {name: 'artifact-recover-col', subscriptions: ['artifact-bad', a.name], ignoreFailedRemoteSub: 'fallbackQuiet'}], 'collections');
      const item = $substore.read('subs').find(sub => sub.name === 'artifact-bad');
      item.process = [inline('throw new Error("collection child failure")')];
      const result = await produce({type: 'col', name: 'artifact-recover-col', includeRaw: true});
      equal(names(JSON.parse(result.result)), [a.name], 'successful source retained');
      equal(result.raw['artifact-bad'], [], 'failed raw cleared by collection fallback');
      return result;
    });
    await check('PC118-artifact-file-raw-merge-and-all', async () => {
      const file = {name: 'artifact-text', source: 'local', content: '文件🙂\r\nzero\u0000', process: [inline('$content += "|" + String($options.zero); $options.flag = false;')]};
      const result = await produce({type: 'file', file, all: true, $options: {zero: 0}});
      equal([result.$content, result.$files, result.$options], [file.content + '|0', [file.content], {zero: 0, flag: false}], 'file shortcut and typed options');
      const raw = await produce({type: 'file', file, produceType: 'raw'});
      equal(raw, JSON.stringify([file.content]), 'raw before processing');
      const zero = await produce({type: 'file', file: {source: 'local', content: 0}, all: true});
      equal([zero.$content, zero.$files], ['0', [0]], 'zero file content');
      return {result, raw, zero};
    });
    await check('PC119-artifact-remote-file-and-ignore-errors', async () => {
      const file = {name: 'artifact-remote-file', source: 'remote', url: probeUpstream + '/host/artifact/file\n' + probeUpstream + '/host/artifact/bad',
        content: 'local', mergeSources: 'localFirst', ignoreFailedRemoteFile: 'enabled'};
      const before = notifications.length;
      const result = await produce({type: 'file', file, noCache: true, all: true});
      equal(result.$files, ['local', '远程文件🙂\r\nline\n', ''], 'remote source order and ignored error');
      equal(result.$content, 'local\n远程文件🙂\r\nline\n', 'file merge exact bytes');
      equal(notifications.length, before + 1, 'one file error notification');
      return {result, notifications: notifications.slice(before)};
    });
    await check('PC120-artifact-mihomo-profile-and-add-proxies', async () => {
      const file = {name: 'artifact-config', type: 'mihomoProfile', sourceType: 'none', process: [inline('port: 7890\nflag: false\nzero: 0'),
        {type: 'Add Proxies From Subscription Operator', args: {sourceType: 'subscription', sourceName: a.name}}]};
      const result = await produce({type: 'file', file, all: true});
      const config = utils.yaml.safeLoad(result.$content);
      equal([config.port, config.flag, config.zero, names(config.proxies)], [7890, false, 0, [a.name]], 'actual config patch and artifact operator');
      equal(file.type, 'mihomoProfile', 'legacy source not mutated');
      const fromCollection = utils.yaml.safeLoad(await produce({type: 'file', file: {name: 'artifact-from-col', type: 'mihomoConfig', sourceType: 'collection', sourceName: 'artifact-remote-col'}, noFlow: true}));
      equal(names(fromCollection.proxies), ['slow', 'remote'], 'config recursively generates collection');
      return {result, config, fromCollection};
    });
    await check('PC121-artifact-nested-native-http-and-script', async () => {
      const nested = {name: 'artifact-nested', source: 'remote', url: probeUpstream + '/host/artifact/file', process: [inline(`async function operator(input) {
        await $substore.wait(1);
        const result = JSON.parse(await produceArtifact({type: 'sub', subscription: {name: 'nested-sub', url: $arguments.url}, noCache: true}));
        input.$content += '|' + result[0].name;
        return input;
      }`, {url: probeUpstream + '/host/artifact/sub'})]};
      const result = await operator(`async function operator(proxies) {
        const text = await produceArtifact({type: 'file', file: $arguments.file, noCache: true});
        $substore.write({text, zero: 0, flag: false}, 'artifactHelper');
        return [{name: text}];
      }`, [], {file: nested});
      equal(names(result), ['远程文件🙂\r\nline\n|remote'], 'actual nested helper across native HTTP/timer awaits');
      return result;
    });
    await check('PC122-artifact-internal-file-download', async () => {
      const result = await utils.download('/api/file/artifact-script');
      equal(result, $substore.read('files')[0].content, 'download generates a real file');
      const generated = await produce({type: 'file', file: {name: 'artifact-sub-config', type: 'mihomoConfig', sourceType: 'subscription', sourceName: a.name}});
      equal(names(utils.yaml.safeLoad(generated).proxies), [a.name], 'file uses real subscription');
      return {result, generated};
    });
    await check('PC123-artifact-errors-and-recovery', async () => {
      const errors = [];
      for (const [type, part] of [['sub', '找不到订阅'], ['col', '找不到组合订阅'], ['file', '找不到文件'], ['rule', '找不到规则']]) errors.push(await rejects(() => produce({type, name: 'missing'}), part));
      errors.push(await rejects(() => produce({type: 'sub'}), '未提供订阅名称或订阅数据'));
      errors.push(await rejects(() => utils.process([], [link('/api/module/missing')]), '无法加载 module'));
      errors.push(await rejects(() => utils.process([], [link('/api/file/missing')]), '无法加载 file'));
      // The pinned rule branch calls the absent RuleUtils.process; preserve its failure.
      errors.push(await rejects(() => produce({type: 'rule', name: 'artifact-rule'}), '', TypeError));
      equal(await produce({type: 'unknown'}), undefined, 'unknown type retains undefined');
      equal(names(JSON.parse(await produce({type: 'sub', name: a.name}))), [a.name], 'helper recovers after errors');
      return errors;
    });
    await check('PC124-artifact-pending-reentry-and-persistence', async () => {
      const result = await probeReenter(JSON.stringify({name: a.name}), async payload => {
        const request = JSON.parse(payload);
        const result = await Promise.all([produce({type: 'sub', name: request.name}), produce({type: 'file', name: 'artifact-script'})]);
        return JSON.stringify(result);
      });
      equal(names(JSON.parse(JSON.parse(result)[0])), [a.name], 'real artifact callback during active native reentry');
      const persisted = $substore.read('artifactHelper');
      equal(persisted, {text: '远程文件🙂\r\nline\n|remote', zero: 0, flag: false}, 'nested result persisted with falsy types');
      $substore.persistCache();
      return {result: JSON.parse(result), persisted};
    });
  } finally {
    if (savedNotify === undefined) delete $substore.notify; else $substore.notify = savedNotify;
  }
};
