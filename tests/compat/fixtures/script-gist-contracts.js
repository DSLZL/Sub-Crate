async function runGistContracts(check, host, operator, assert, equal) {
  const Gist = host.ProxyUtils.Gist;
  const base = probeUpstream + '/host/gist';
  const saved = $substore.read('settings');
  const copy = value => JSON.parse(JSON.stringify(value));
  const settings = values => $substore.write({...saved, githubApiUrl: base + '/github', githubApiTimeout: 1000, ...values}, 'settings');
  const create = (key = 'helper', syncPlatform = 'github') => {
    const gist = new Gist({token: 'helper-token', key, syncPlatform});
    if (syncPlatform === 'gitlab') {
      // The fixed GitLab origin is fixture-routed before the original queued transport.
      for (const method of ['get', 'post', 'put', 'patch', 'delete']) {
        const send = gist.http[method];
        gist.http[method] = input => {
          const options = typeof input === 'string' ? {url: input} : input;
          options.url = base + '/gitlab' + options.url;
          return send(options);
        };
      }
    }
    return gist;
  };
  async function rejects(callback, expected) {
    let caught = false;
    try { await callback(); } catch (error) {
      caught = true;
      if (typeof expected === 'string') equal(String(error), expected, 'exact Gist rejection');
      else assert(error instanceof expected, 'native error prototype');
    }
    assert(caught, 'Gist operation must reject');
    return true;
  }
  try {
    settings();
    await check('PC77-gist-constructor-auth-and-methods', () => {
      const github = create(), gitlab = create('helper', 'gitlab');
      equal(github.headers.Authorization, 'token helper-token', 'GitHub auth');
      equal(gitlab.headers['PRIVATE-TOKEN'], 'helper-token', 'GitLab auth');
      equal(github.headers['User-Agent'], gitlab.headers['User-Agent'], 'shared UA');
      assert(!Object.hasOwn(github.headers, 'PRIVATE-TOKEN') && !Object.hasOwn(gitlab.headers, 'Authorization'), 'separate credentials');
      for (const method of ['locate', 'upload', 'download']) assert(typeof github[method] === 'function', method);
      let rejected = false;
      try { new Gist(); } catch (error) { rejected = error instanceof TypeError; }
      assert(rejected, 'constructor requires options');
      equal(create('helper', 'unknown').headers.Authorization, 'token helper-token', 'unknown platform uses GitHub');
      return {methods: ['locate', 'upload', 'download'], auth: true, missingOptions: rejected};
    });
    await check('PC78-gist-missing-locate-and-empty-input', async () => {
      await $substore.http.post({url: base + '/reset'});
      const gist = create();
      assert(await gist.locate() === undefined, 'missing locate');
      await rejects(() => gist.download('missing'), '找不到 Sub-Store Gist (helper)');
      await rejects(() => gist.upload({}), '未提供需上传的文件');
      return {missing: true, empty: true};
    });
    await check('PC79-gist-create-filter-and-unicode-download', async () => {
      await create('other').upload({other: {content: 'other'}});
      const input = {'订阅🙂.txt': {content: '订阅🙂\r\nsecond\n'}, 'remove.txt': {content: 'remove'}, empty: {content: ''}, nil: {content: null}};
      const before = copy(input);
      const response = await create().upload(input);
      equal(response.statusCode, 201, 'create status');
      equal(Object.keys(JSON.parse(response.body).files), ['订阅🙂.txt', 'remove.txt'], 'empty files filtered');
      equal(input, before, 'create input retained');
      const text = await create().download('订阅🙂.txt');
      equal(text, '订阅🙂\r\nsecond\n', 'raw Unicode/line endings');
      return {status: response.statusCode, text};
    });
    await check('PC80-gist-update-delete-and-input-mutation', async () => {
      const input = {'订阅🙂.txt': {content: 'changed'}, 'remove.txt': {content: ''}, zero: {content: '0'}, ghost: {content: null}};
      const response = await create().upload(input);
      equal(response.statusCode, 200, 'update status');
      equal(Object.keys(JSON.parse(response.body).files), ['订阅🙂.txt', 'zero'], 'files updated');
      equal(input, {'订阅🙂.txt': {content: 'changed'}, 'remove.txt': null, zero: {content: '0'}}, 'legacy update mutates input');
      return {status: response.statusCode, input};
    });
    await check('PC81-gist-empty-file-fallback-lifecycle', async () => {
      const options = {emptyFileFallback: {filename: 'keep', content: 'placeholder'}};
      const gist = create();
      const created = await gist.upload({'订阅🙂.txt': {content: null}, zero: {content: ''}}, options);
      const retained = await gist.upload({ghost: {content: ''}}, options);
      const removed = await gist.upload({real: {content: 'restored'}}, options);
      const results = [created, retained, removed].map(value => value.subStoreUploadMeta);
      equal(results, ['created', 'retained', 'removed'].map(status => ({emptyFileFallback: {status, filename: 'keep'}})), 'placeholder metadata');
      equal(Object.keys(JSON.parse(removed.body).files), ['real'], 'placeholder deleted');
      return results;
    });
    await check('PC82-gist-validation-before-provider-write', async () => {
      await rejects(() => create().upload({real: {content: ''}}), '本次操作将导致所有文件的内容都为空, 无法更新 gist');
      await rejects(() => create('empty').upload({empty: {content: ''}}), '所有文件的内容都为空, 无法创建 gist');
      await rejects(() => create('empty').upload({nil: {content: null}}, {emptyFileFallback: {filename: 'keep'}}), '所有文件的内容都为空, 无法创建 gist');
      await rejects(() => create().upload(null), TypeError);
      await rejects(() => create().upload({broken: null}), TypeError);
      return {emptyUpdate: true, emptyCreate: true, noCreateFallback: true, types: true};
    });
    await check('PC83-gist-provider-errors-timeout-and-recovery', async () => {
      const gist = create();
      const messages = ['ERROR: HTTP 400: nested', 'ERROR: HTTP 401: second', 'ERROR: HTTP 429: third',
        'ERROR: HTTP 503: raw fixture error', 'ERROR: HTTP 404: ', 'ERROR: HTTP 500: [object Object]'];
      for (const [index, mode] of ['nested', 'error', 'message', 'text', 'empty', 'object'].entries()) {
        await rejects(() => gist.http.get(base + '/errors/' + mode), messages[index]);
      }
      settings({githubApiUrl: base + '/errors/invalid'});
      await rejects(() => create().locate(), SyntaxError);
      settings();
      let timeout;
      try { await gist.http.get({url: base + '/errors/delay', timeout: 20}); }
      catch (error) { timeout = error; }
      assert(timeout instanceof Error && timeout.message === `GET URL: ${base}/errors/delay exceeds the timeout 20 ms`, 'exact timeout');
      await $substore.wait(230);
      equal((await gist.locate()).id, 'g2', 'queue recovers after errors/timeout');
      return {messages, syntaxError: true, timeout: true, recovered: true};
    });
    await check('PC84-gist-five-method-queue-and-rejection-recovery', async () => {
      const gist = create();
      const result = await Promise.all(['get', 'post', 'put', 'patch', 'delete'].map(method => gist.http[method](base + '/queue/' + method)));
      const values = result.map(response => JSON.parse(response.body));
      equal(values, ['get', 'post', 'put', 'patch', 'delete'].map(name => ({name, active: 1})), 'five methods share one queue');
      await rejects(() => gist.http.get(base + '/queue/rejected'), 'ERROR: HTTP 503: queue rejected');
      equal(JSON.parse((await gist.http.get(base + '/queue/recovery')).body), {name: 'recovery', active: 1}, 'rejection releases queue');
      return {values, recovered: true};
    });
    await check('PC85-gitlab-create-update-and-file-limits', async () => {
      const gist = create('snippet', 'gitlab');
      assert(await gist.locate() === undefined, 'missing snippet');
      equal((await gist.upload({alpha: {content: '一🙂'}, gone: {content: 'delete'}})).statusCode, 201, 'snippet create');
      const input = {alpha: {content: 'changed'}, beta: {content: 'second'}, gone: {content: ''}, ghost: {content: ''}};
      const response = await gist.upload(input);
      equal(JSON.parse(response.body).files, [{path: 'alpha'}, {path: 'beta'}], 'snippet actions');
      equal(input, {alpha: {content: 'changed'}, beta: {content: 'second'}, gone: null}, 'snippet input mutation');
      await rejects(() => gist.upload(Object.fromEntries(Array.from({length: 9}, (_, index) => ['new' + index, {content: 'x'}]))), '本次操作将导致 snippet 的文件数超过 10, 无法更新 snippet');
      await rejects(() => gist.upload({alpha: {content: ''}, beta: {content: ''}}), '本次操作将导致所有文件的内容都为空, 无法更新 snippet');
      await rejects(() => create('empty', 'gitlab').upload({empty: {content: ''}}), '所有文件的内容都为空, 无法创建 snippet');
      return {created: true, updated: true, fileLimit: true, input};
    });
    await check('PC86-gitlab-fallback-and-legacy-download-path', async () => {
      const gist = create('snippet', 'gitlab');
      const options = {emptyFileFallback: {filename: 'keep', content: 'placeholder'}};
      const responses = [await gist.upload({alpha: {content: null}, beta: {content: ''}}, options),
        await gist.upload({ghost: {content: ''}}, options), await gist.upload({real: {content: 'restored'}}, options)];
      const statuses = responses.map(response => response.subStoreUploadMeta.emptyFileFallback.status);
      equal(statuses, ['created', 'retained', 'removed'], 'snippet fallback metadata');
      await rejects(() => gist.download('real'), 'ERROR: HTTP 404: fixture resource missing');
      await rejects(() => create('empty', 'gitlab').upload({nil: {content: null}}, options), '所有文件的内容都为空, 无法创建 snippet');
      return {statuses, legacyDownloadError: true, noCreateFallback: true};
    });
    await check('PC87-gist-in-async-script-and-persistence', async () => {
      const result = await operator(`async function operator(proxies) {
        const gist = new ProxyUtils.Gist({token: 'helper-token', key: 'helper'});
        const text = await gist.download('real');
        $substore.write({text, zero: 0, flag: false}, 'gistHelper');
        return proxies.map(proxy => ({...proxy, name: proxy.name + ':' + text}));
      }`, [{name: 'script', unknown: null}]);
      equal(result, [{name: 'script:restored', unknown: null}], 'injected Gist reaches HTTP');
      equal($substore.read('gistHelper'), {text: 'restored', zero: 0, flag: false}, 'persisted Gist result');
      return result;
    });
    await check('PC88-gist-http-defaults-shallow-overrides-and-events', async () => {
      const gist = create();
      const options = {url: '/gists?per_page=100&page=1', requestId: 'gist-defaults', headers: {
        Authorization: 'token helper-token', 'User-Agent': 'Gist override/1', 'X-Gist': 'yes'}, events: {
        onRequest(method, options) { equal(method, 'GET', 'hook method'); options.headers['X-Gist-Event'] = 'mutated'; },
        async onResponse(response) { await $substore.wait(1); response.headers['x-gist-response'] = 'mutated'; return {ignored: true}; },
      }};
      const response = await gist.http.get(options);
      equal(options.url, base + '/github/gists?per_page=100&page=1', 'relative URL mutates caller');
      equal(response.requestId, 'gist-defaults', 'explicit request identity');
      equal(response.headers['x-gist-response'], 'mutated', 'async response hook');
      equal(JSON.parse(response.body).length, 2, 'original response retained');
      settings({githubApiTimeout: 0});
      let defaultTimeout;
      await create().http.get({url: '/gists?per_page=100&page=1', events: {onRequest(method, options) {defaultTimeout = options.timeout;}}});
      equal(defaultTimeout, 10000, 'falsy timeout uses default');
      return {relativeMutation: true, requestId: response.requestId, shallowHeaders: true, asyncResponse: true, defaultTimeout};
    });
  } finally { $substore.write(saved, 'settings'); }
}
