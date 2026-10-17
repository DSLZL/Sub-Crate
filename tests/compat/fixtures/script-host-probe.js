// Only the experiment's required surface; production semantics remain an M3 gate.
const $arguments = { onlyFlagIP: true };
const $options = {};
const b64d = b64decode;
const b64e = b64encode;
const context = { env: { backend: 'Rust', version: '2.42.4' } };
// shortcut: only the fixture's IPv4 path; replace with full ProxyUtils before M3.
const ProxyUtils = { isIP: value => /^(?:\d{1,3}\.){3}\d{1,3}$/.test(value) };
const $substore = {
  env: { isNode: false },
  read: key => { const value = __read(key); return value == null ? undefined : JSON.parse(value); },
  write: (value, key) => __write(JSON.stringify(value), key),
  wait: __wait,
  http: { get: async options => JSON.parse(await __httpGet(
    options.url.replace('http://ip-api.com', probeUpstream), JSON.stringify(options.headers || {})
  )) },
};
let nextTimerId = 1;
const timers = new Map();
// shortcut: one-shot timers only; verify interval/overflow/error semantics before M3.
function setTimeout(callback, milliseconds = 0, ...arguments_) {
  if (typeof callback !== 'function') throw new TypeError('Timer callback must be a function');
  const id = nextTimerId++;
  timers.set(id, true);
  __wait(Math.max(0, Math.trunc(Number(milliseconds) || 0))).then(() => {
    if (timers.delete(id)) callback(...arguments_);
  });
  return id;
}
function clearTimeout(id) { timers.delete(id); }
function produceArtifact(callback) { return __nested(callback); }
