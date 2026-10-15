// Development oracle: actual processor exports and application host, not the extracted bundle.
const fs = require('node:fs');
const path = require('node:path');
const fixture = name => fs.readFileSync(path.join(__dirname, 'fixtures', name), 'utf8');
eval(fixture('script-processor-clock.js'));
eval(fixture('script-helper-contracts.js'));
eval(fixture('script-dns-contracts.js'));
const moduleSource = require(path.resolve(__dirname, '../../backend/src/core/proxy-utils/processors/index.js'));
const $substore = require(path.resolve(__dirname, '../../backend/src/core/app.js')).default;
const processorHost = {
  factories: moduleSource.default,
  ApplyProcessor: moduleSource.ApplyProcessor,
  ApplyResponseTransformer: moduleSource.ApplyResponseTransformer,
  scriptResourceCache: require(path.resolve(__dirname, '../../backend/src/utils/script-resource-cache.js')).default,
  headersResourceCache: require(path.resolve(__dirname, '../../backend/src/utils/headers-resource-cache.js')).default,
  resourceCache: require(path.resolve(__dirname, '../../backend/src/utils/resource-cache.js')).default,
  DOMAIN_RESOLVERS: moduleSource.default['Resolve Domain Operator'].resolver,
  YAML: require(path.resolve(__dirname, '../../backend/src/utils/yaml.js')).default,
  ProxyUtils: require(path.resolve(__dirname, '../../backend/src/core/proxy-utils/index.js')).ProxyUtils,
  flowUtils: require(path.resolve(__dirname, '../../backend/src/utils/flow.js')),
};
const probeUpstream = process.env.SUB_STORE_COMPAT_UPSTREAM;
const probeKind = 'node';
const probeReenter = async (payload, callback) => {
  await $substore.wait(1);
  return await callback(payload);
};
const script = process.argv.includes('--resume')
  ? 'Promise.resolve({storage: {cache: $substore.cache, root: $substore.root}, resourceCache: processorHost.scriptResourceCache.resourceCache, headersCache: processorHost.headersResourceCache.resourceCache, dnsCache: processorHost.resourceCache.resourceCache})'
  : fixture('script-processor-contracts.js');
eval(script).then(result => {
  fs.writeSync(1, `COMPAT_PROCESSOR_RESULT ${JSON.stringify(result)}\n`);
}, error => {
  fs.writeSync(2, `${error.stack}\n`);
  process.exitCode = 1;
});
