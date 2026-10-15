// Source registries only use diagnostics; resolve the existing host after setup.
export default new Proxy({}, {get: (_target, key) => $substore[key]});
