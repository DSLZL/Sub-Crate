// Contained M1 adapter. Rust owns filesystem/network I/O; JS preserves object identity.
const hostState = JSON.parse(__loadHostState());
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
