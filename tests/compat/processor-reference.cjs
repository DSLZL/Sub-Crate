// Development oracle: actual processor exports and application host, not the extracted bundle.
const fs = require('node:fs');
const path = require('node:path');
const fixture = name => fs.readFileSync(path.join(__dirname, 'fixtures', name), 'utf8');
process.env.SUB_STORE_FRONTEND_BACKEND_PATH = '/';
eval(fixture('script-processor-clock.js'));
eval(fixture('script-helper-contracts.js'));
eval(fixture('script-dns-contracts.js'));
eval(fixture('script-native-dns-contracts.js'));
eval(fixture('script-mmdb-contracts.js'));
eval(fixture('script-gist-contracts.js'));
eval(fixture('script-conversion-contracts.js'));
eval(fixture('script-artifact-contracts.js'));
const moduleSource = require(path.resolve(__dirname, '../../backend/src/core/proxy-utils/processors/index.js'));
const $substore = require(path.resolve(__dirname, '../../backend/src/core/app.js')).default;
const processorHost = {
  produceArtifact: require(path.resolve(__dirname, '../../backend/src/restful/sync.js')).produceArtifact,
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
  ...require(path.resolve(__dirname, '../../backend/src/utils/dns.js')),
};
const probeUpstream = process.env.SUB_STORE_COMPAT_UPSTREAM;
const probeDnsServers = JSON.parse(process.env.SUB_STORE_COMPAT_DNS_SERVERS || '{}');
const probeKind = 'node';
const probeCaseProgress = (id, event) => fs.writeSync(2, `COMPAT_PROCESSOR ${JSON.stringify({id, event})}\n`);
const probeMmdbRoot = process.env.SUB_STORE_COMPAT_STATE_DIR;
const probeMmdbVectors = JSON.parse(fs.readFileSync(path.join(probeMmdbRoot, 'mmdb-vectors.json'), 'utf8'));
const probeMmdbRemoveTransient = () => fs.unlinkSync(path.join(probeMmdbRoot, 'mmdb transient🙂.mmdb'));
const probeMmdbWithDefaults = (values, callback) => {
  const keys = {country: 'SUB_STORE_MMDB_COUNTRY_PATH', asn: 'SUB_STORE_MMDB_ASN_PATH'};
  const saved = Object.fromEntries(Object.values(keys).map(key => [key, process.env[key]]));
  for (const [key, name] of Object.entries(keys)) {
    if (values[key] == null) delete process.env[name]; else process.env[name] = values[key];
  }
  try { return callback(); } finally {
    for (const [key, value] of Object.entries(saved)) {
      if (value === undefined) delete process.env[key]; else process.env[key] = value;
    }
  }
};
const probeReenter = async (payload, callback) => {
  await $substore.wait(1);
  return await callback(payload);
};
const script = process.argv.includes('--resume')
  ? 'Promise.resolve({storage: {cache: $substore.cache, root: $substore.root}, resourceCache: processorHost.scriptResourceCache.resourceCache, headersCache: processorHost.headersResourceCache.resourceCache, dnsCache: processorHost.resourceCache.resourceCache})'
  : fixture('script-processor-contracts.js');
eval(script).then(result => {
  process.stdout.write(`COMPAT_PROCESSOR_RESULT ${JSON.stringify(result)}\n`);
}, error => {
  fs.writeSync(2, `${error.stack}\n`);
  process.exitCode = 1;
});
