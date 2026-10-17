// Contained M1 adapter. Rust owns filesystem/network I/O; JS preserves object identity.
const hostState = JSON.parse(__loadHostState());
let hostTimerId = 1;
const hostTimers = new Map();
function hostDelay(value) {
  const number = Number(value);
  return number >= 1 && number <= 2147483647 ? Math.trunc(number) : 1;
}
function setTimeout(callback, delay, ...args) {
  if (typeof callback !== 'function') throw new TypeError('Timer callback must be a function');
  const id = hostTimerId++;
  hostTimers.set(id, true);
  __hostWait(hostDelay(delay)).then(() => { if (hostTimers.delete(id)) callback(...args); });
  return id;
}
function clearTimeout(id) { hostTimers.delete(id); }
function setInterval(callback, delay, ...args) {
  if (typeof callback !== 'function') throw new TypeError('Timer callback must be a function');
  const id = hostTimerId++;
  hostTimers.set(id, true);
  (async () => {
    while (hostTimers.has(id)) {
      await __hostWait(hostDelay(delay));
      if (hostTimers.has(id)) callback(...args);
    }
  })();
  return id;
}
function clearInterval(id) { clearTimeout(id); }

let hostRequestId = 0;
function createHostHTTP(defaultOptions = {baseURL: ''}) {
  // Preserve the source client's shallow merge and legacy absolute-URL test.
  const absolute = /https?:\/\/(www\.)?[-a-zA-Z0-9@:%._+~#=]{1,256}\.[a-zA-Z0-9()]{1,6}\b([-a-zA-Z0-9()@:%_+.~#?&//=]*)/;
  const send = (method, input) => {
    const options = typeof input === 'string' ? {url: input} : input;
    if (defaultOptions.baseURL && !absolute.test(options.url || '')) options.url = defaultOptions.baseURL + options.url;
    return hostSend(method, {...defaultOptions, ...options});
  };
  const http = {request: options => send(options.method || 'GET', options)};
  for (const method of ['GET', 'POST', 'PUT', 'DELETE', 'HEAD', 'OPTIONS', 'PATCH']) {
    http[method.toLowerCase()] = options => send(method, options);
  }
  return http;
}
async function hostSend(method, input) {
  const options = typeof input === 'string' ? {url: input} : input;
  const requestId = options.requestId || `${Date.now()}-${++hostRequestId}`;
  const events = {onRequest() {}, onResponse(value) { return value; }, onTimeout() {}, ...options.events};
  events.onRequest(method, options);
  // Match the reference's setup error before installing its timeout.
  const opts = JSON.parse(JSON.stringify(options));
  const headers = Object.fromEntries(Object.entries(opts.headers || {}).map(([key, value]) => [key.toLowerCase(), value]));
  if (!Object.hasOwn(headers, 'accept')) headers.accept = '*/*';
  if (headers.accept == null) delete headers.accept;
  let url = opts.url;
  const credentials = /^(https?:\/\/)([^/@]*)@(.*)$/.exec(url);
  if (credentials) {
    const separator = credentials[2].indexOf(':');
    const user = separator < 0 ? credentials[2] : credentials[2].slice(0, separator);
    const password = separator < 0 ? '' : credentials[2].slice(separator + 1);
    headers.authorization = `Basic ${b64encode(`${decodeURIComponent(user)}:${decodeURIComponent(password)}`)}`;
    url = credentials[1] + credentials[3];
  }
  let body;
  if (options.body != null) {
    const value = options.body;
    if (typeof value === 'string') body = Array.from(Buffer.from(value));
    else if (value instanceof ArrayBuffer) body = Array.from(new Uint8Array(value));
    else if (ArrayBuffer.isView(value)) body = Array.from(new Uint8Array(value.buffer, value.byteOffset, value.byteLength));
    else throw new TypeError('Unsupported HTTP body');
  }
  const worker = __hostRequest(method.toUpperCase(), url, JSON.stringify(headers), body == null ? undefined : JSON.stringify(body)).then(raw => {
    const response = JSON.parse(raw);
    const bytes = Uint8Array.from(response.bytes);
    return {statusCode: response.statusCode, headers: response.headers,
      body: opts.encoding === null ? bytes.buffer : Buffer.from(bytes).toString('utf8'), requestId};
  });
  return new Promise((resolve, reject) => {
    let settled = false;
    let settling = false;
    let timer;
    const finish = (callback, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      callback(value);
    };
    worker.then(async value => {
      if (settled || settling) return;
      settling = true;
      try { await events.onResponse(value); finish(resolve, {...value, requestId}); }
      catch (error) { finish(reject, error); }
    }, error => finish(reject, error));
    if (options.timeout) timer = setTimeout(() => {
      if (settled) return;
      try { events.onTimeout(); } catch (_) {}
      finish(reject, new Error(`${method} URL: ${options.url} exceeds the timeout ${options.timeout} ms`));
    }, options.timeout);
  });
}

const $substore = {
  cache: hostState.cache,
  root: hostState.root,
  env: {isNode: false},
  persistCache() { __persistHostState(JSON.stringify(this.cache, null, 2), JSON.stringify(this.root, null, 2)); },
  read(key) { return key.indexOf('#') !== -1 ? this.root[key.substr(1)] : this.cache[key]; },
  write(value, key) {
    if (key.indexOf('#') !== -1) this.root[key.substr(1)] = value;
    else this.cache[key] = value;
    this.persistCache();
  },
  delete(key) {
    if (key.indexOf('#') !== -1) delete this.root[key.substr(1)];
    else delete this.cache[key];
    this.persistCache();
  },
  wait(delay) { return new Promise(resolve => setTimeout(resolve, delay)); },
  http: createHostHTTP(),
};
// shortcut: proxy/TLS/compression/DNS/notifications, callback error handling and
// application artifact calls need shared contracts before M3 production use.
