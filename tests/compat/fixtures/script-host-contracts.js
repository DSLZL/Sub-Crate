(async () => {
  const passed = [];
  function assert(condition, message) { if (!condition) throw new Error(message); }
  function equal(actual, expected, message) { assert(JSON.stringify(actual) === JSON.stringify(expected), message); }
  async function rejects(action, pattern, message) {
    let caught;
    try { await action(); } catch (error) { caught = error; }
    assert(caught instanceof Error && pattern.test(caught.message), message);
  }
  const value = {zero: 0, flag: false, nil: null, empty: '', text: '订阅🙂\r\nsecond\n', script: "require('node:fs')\r\n"};
  assert($substore.read('missing') === undefined, 'missing read must be undefined');
  assert($substore.write(value, 'fixture') === undefined, 'write must return undefined');
  equal($substore.read('fixture'), value, 'stored fields and bytes');
  assert($substore.read('fixture') === value, 'write/read object identity');
  $substore.read('fixture').mutated = false;
  $substore.persistCache();
  passed.push('SH01-storage-values-and-aliases');

  assert($substore.write({root: false}, '#root-fixture') === undefined, 'root write result');
  assert($substore.read('#root-fixture') === $substore.root['root-fixture'], 'root identity');
  $substore.write(0, 'odd#key');
  assert($substore.root['dd#key'] === 0 && !Object.hasOwn($substore.cache, 'odd#key'), 'historical hash key slicing');
  assert($substore.delete('odd#key') === undefined && $substore.read('odd#key') === undefined, 'root delete');
  passed.push('SH02-root-key-routing');

  $substore.write(undefined, 'undefined');
  assert(Object.hasOwn($substore.cache, 'undefined') && $substore.read('undefined') === undefined, 'in-memory undefined property');
  assert($substore.delete('undefined') === undefined && !Object.hasOwn($substore.cache, 'undefined'), 'cache delete result');
  const circular = {}; circular.self = circular;
  await rejects(() => $substore.write(circular, 'circular'), /circular|cyclic/i, 'circular persistence must reject');
  $substore.delete('circular');
  passed.push('SH03-storage-undefined-delete-and-errors');

  for (const method of ['GET', 'POST', 'PUT', 'DELETE', 'HEAD', 'OPTIONS', 'PATCH']) {
    const response = await $substore.http[method.toLowerCase()]({url: probeUpstream + '/host/echo',
      requestId: 'fixed-' + method, headers: {'X-Probe': method},
      ...(['POST', 'PUT', 'DELETE', 'OPTIONS', 'PATCH'].includes(method) ? {body: value.text} : {})});
    equal(Object.keys(response).sort(), ['body', 'headers', 'requestId', 'statusCode'], method + ' response keys');
    assert(response.statusCode === 200 && response.requestId === 'fixed-' + method, method + ' status/id');
    assert(response.headers['x-fixture'] === 'host-contract', method + ' response header');
    if (method === 'HEAD') assert(response.body === '', 'HEAD empty body');
    else equal(JSON.parse(response.body), {method, body: ['GET'].includes(method) ? [] : Array.from(Buffer.from(value.text)),
      accept: '*/*', probe: method, authorization: null}, method + ' outgoing wire data');
  }
  passed.push('SH04-all-http-methods');

  const binary = await $substore.http.get({url: probeUpstream + '/host/binary', encoding: null});
  assert(binary.body instanceof ArrayBuffer, 'binary response type');
  equal(Array.from(new Uint8Array(binary.body)), [0, 255, 195, 40, 13, 10], 'binary response bytes');
  equal(binary.headers['set-cookie'], ['first=1', 'second=2'], 'repeated cookies');
  assert(/^\d+-\d+$/.test(binary.requestId), 'generated request ID shape');
  const text = await $substore.http.get(probeUpstream + '/host/text');
  assert(text.body === value.text, 'HTTP Unicode and CRLF text');
  passed.push('SH05-binary-text-headers-and-string-url');

  for (const body of [Buffer.from([0, 255, 13, 10]), Uint8Array.from([9, 0, 255, 13, 10, 8]).subarray(1, 5)]) {
    const response = await $substore.http.request({method: 'post', url: probeUpstream + '/host/echo',
      headers: {Accept: null}, body});
    equal(JSON.parse(response.body), {method: 'POST', body: [0, 255, 13, 10], accept: null, probe: null, authorization: null}, 'binary request view/accept removal');
  }
  const credentials = probeUpstream.replace('http://', 'http://user%40name:p%40ss%3A%2520@');
  const auth = await $substore.http.get({url: credentials + '/host/echo', headers: {Accept: 'application/json'}});
  equal(JSON.parse(auth.body), {method: 'GET', body: [], accept: 'application/json', probe: null,
    authorization: 'Basic ' + Buffer.from('user@name:p@ss:%20').toString('base64')}, 'decoded URL credentials');
  passed.push('SH06-binary-request-accept-and-auth');

  const order = [];
  const hooked = await $substore.http.get({url: probeUpstream + '/host/echo', events: {
    onRequest(method, options) { order.push(method); options.headers = {'X-Probe': 'hook'}; },
    async onResponse(response) { await $substore.wait(2); order.push('response'); response.body = 'mutated'; return {body: 'ignored'}; },
  }});
  assert(hooked.body === 'mutated', 'onResponse mutation retained, return ignored');
  equal(order, ['GET', 'response'], 'async event order');
  await rejects(() => $substore.http.get({url: probeUpstream + '/host/echo', events: {
    async onResponse() { await $substore.wait(1); throw new Error('response-hook-error'); },
  }}), /^response-hook-error$/, 'response hook rejection');
  await rejects(() => $substore.http.get({url: probeUpstream + '/host/never', events: {
    onRequest() { throw new Error('request-hook-error'); },
  }}), /^request-hook-error$/, 'request hook rejection');
  passed.push('SH07-request-and-async-response-events');

  let timeoutCalls = 0;
  let lateResponses = 0;
  await rejects(() => $substore.http.get({url: probeUpstream + '/host/delay', timeout: '30', events: {
    onTimeout() { timeoutCalls++; throw new Error('ignored-timeout-hook'); },
    onResponse() { lateResponses++; },
  }}), /GET URL: .*\/host\/delay exceeds the timeout 30 ms/, 'timeout envelope');
  assert(timeoutCalls === 1, 'timeout hook once');
  let finishedResponse = false;
  await rejects(() => $substore.http.get({url: probeUpstream + '/host/echo', timeout: 30, events: {
    async onResponse() { await $substore.wait(100); finishedResponse = true; }, onTimeout() { timeoutCalls++; },
  }}), /exceeds the timeout 30 ms/, 'timeout while response hook pending');
  await $substore.wait(250);
  assert(timeoutCalls === 2 && finishedResponse && lateResponses === 0, 'late completion must not settle or invoke a response hook again');
  passed.push('SH08-timeout-hooks-and-late-response');

  let setupTimeouts = 0;
  const setup = {url: probeUpstream + '/host/never', timeout: 5, events: {onTimeout() { setupTimeouts++; }}};
  setup.circular = setup;
  await rejects(() => $substore.http.get(setup), /circular|cyclic/i, 'circular request setup rejects');
  await $substore.wait(10);
  assert(setupTimeouts === 0, 'setup failure installs no timer');
  await rejects(() => $substore.http.get('invalid-url'), /url|builder/i, 'invalid URL rejects');
  const failedStatus = await $substore.http.get(probeUpstream + '/host/status');
  assert(failedStatus.statusCode === 503 && failedStatus.body === 'provider rejected', 'HTTP status must not throw');
  passed.push('SH09-setup-errors-and-non-success-status');

  const redirected = await $substore.http.get(probeUpstream + '/host/redirect/2');
  assert(redirected.body === value.text, 'two redirects allowed');
  // Pinned Undici rejects the third follow even when that response is already 200.
  await rejects(() => $substore.http.get(probeUpstream + '/host/redirect/3'), /redirect/i, 'third follow rejects final success');
  await rejects(() => $substore.http.get(probeUpstream + '/host/redirect/4'), /redirect/i, 'long redirect chain rejects');
  passed.push('SH10-redirect-limit');

  const calls = [];
  const canceled = setTimeout(() => calls.push('canceled'), 2);
  clearInterval(canceled);
  await new Promise(resolve => setTimeout((first, second) => { calls.push(first, second); resolve(); }, '2', 0, false));
  await new Promise(resolve => {
    const timer = setInterval(value => { calls.push(value); if (calls.length === 5) { clearTimeout(timer); resolve(); } }, 2, 'tick');
  });
  await $substore.wait(10);
  equal(calls, [0, false, 'tick', 'tick', 'tick'], 'timer arguments, intervals and cross-cancellation');
  assert(await $substore.wait(1) === undefined, 'wait return type');
  await rejects(() => setTimeout(null, 1), /callback/i, 'timer callback validation');
  passed.push('SH11-timer-interval-arguments-and-cancellation');
  return {passed, storage: {cache: $substore.cache, root: $substore.root}};
})()
