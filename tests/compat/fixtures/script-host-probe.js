// Only the experiment's required surface; production semantics remain an M3 gate.
const $arguments = { onlyFlagIP: true };
const $options = {};
const context = { env: { backend: 'Rust', version: '2.42.4' } };
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
function setTimeout(callback, milliseconds) {
  return __wait(milliseconds).then(() => callback());
}
function produceArtifact(callback) { return __nested(callback); }
