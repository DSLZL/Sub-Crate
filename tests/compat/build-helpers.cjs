// Development-only bundling of the same utility packages injected by the pinned backend.
const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '../..');
const { createRequire } = require('node:module');
const fromBackend = createRequire(path.join(root, 'backend/package.json'));
const {parseSync} = fromBackend('@babel/core');
// Extract the frozen wrappers verbatim: this tests engine feasibility, not a Rust pipeline.
function declarations(relative, names) {
  const source = fs.readFileSync(path.join(root, relative), 'utf8');
  const nodes = parseSync(source, {babelrc: false, configFile: false, sourceType: 'module'}).program.body.map(node => node.declaration || node);
  return names.map(name => {
    const matches = nodes.filter(node => ['FunctionDeclaration', 'ClassDeclaration'].includes(node.type)
      ? node.id.name === name
      : node.type === 'VariableDeclaration' && node.declarations.length === 1 && node.declarations[0].id.name === name);
    if (matches.length !== 1) throw new Error(`Expected one ${name} in ${relative}`);
    return source.slice(matches[0].start, matches[0].end);
  }).join('\n');
}
const processors = declarations('backend/src/core/proxy-utils/processors/index.js', [
  'trimWrap', 'deepMerge', 'ScriptOperator', 'ScriptFilter', 'ResponseTransformer',
  'ApplyResponseTransformer', 'ApplyFilter', 'ApplyOperator', 'ApplyProcessor', 'clone', 'createDynamicFunction',
]);
const cache = declarations('backend/src/utils/script-resource-cache.js', ['ResourceCache', 'normalizeTTL', 'getTTL']);
const headersCache = declarations('backend/src/utils/headers-resource-cache.js', ['ResourceCache', 'normalizeTTL', 'getTTL']);
const resourceCache = declarations('backend/src/utils/resource-cache.js', ['ResourceCache', 'normalizeTTL', 'getTTL']);
const dns = declarations('backend/src/utils/dns.js', [
  'DNS_DEFAULT_PORT', 'DNS_TLS_DEFAULT_PORT', 'buildDnsQuery', 'doh', 'normalizeDnsTimeout',
  'normalizeDnsHost', 'parseDnsPort', 'assertNoDnsServerUrlExtraParts', 'parseDnsServerUrl', 'parseDnsResolver', 'resolveDns',
]);
const resolvers = declarations('backend/src/core/proxy-utils/processors/index.js', [
  'DEFAULT_RESOLVE_DOMAIN_CUSTOM_DNS_CONCURRENCY', 'normalizeResolveDomainCustomDnsConcurrency',
  'parseCustomDnsUrls', 'resolveWithCustomDnsConcurrency', 'annotateCustomDnsResult',
  'packCustomDnsCachedResult', 'unpackCustomDnsCachedResult', 'DOMAIN_RESOLVERS',
]);
const plainObject = declarations('backend/src/utils/index.js', ['isPlainObject']);
const isIP = declarations('backend/src/core/proxy-utils/index.js', ['isIP']);
const flags = declarations('backend/src/utils/geo.js', ['ISOFlags', 'getFlag', 'getISO', 'removeFlag']);
const mmdb = declarations('backend/src/utils/geo.js', ['MMDB']);
const gist = declarations('backend/src/utils/gist.js', [
  'DEFAULT_GITHUB_API_URL', 'describeGistApiErrorResponse', 'normalizeApiUrl', 'getGithubGistBaseURL', 'Gist',
]);
const download = declarations('backend/src/utils/download.js', [
  'buildDownloadRegex', 'maybePrefixGithubProxyUrl', 'maskDownloadUrl', 'finalizeDownloadedBody',
  'formatDownloadResult', 'formatPlainDownloadResult', 'download',
]);
const findByName = declarations('backend/src/utils/database.js', ['findByName']);
const flow = declarations('backend/src/utils/flow.js', [
  'getFlowField', 'getFlowHeaders', 'parseFlowHeaders', 'flowTransfer', 'validCheck', 'getRmainingDays', 'normalizeFlowHeader',
]);
const concurrency = declarations('backend/src/utils/request-concurrency.js', [
  'DEFAULT_BACKEND_REQUEST_CONCURRENCY', 'BACKEND_REQUEST_CONCURRENCY_SETTING',
  'DEFAULT_BACKEND_REQUEST_CONCURRENCY_WAIT_TIME', 'BACKEND_REQUEST_CONCURRENCY_WAIT_TIME_SETTING',
  'activeRequestCount', 'pendingRequestResolvers', 'sleep', 'normalizeBackendRequestConcurrency',
  'normalizeBackendRequestConcurrencyWaitTime', 'getBackendRequestConcurrency', 'getBackendRequestConcurrencyWaitTime',
  'formatRequestConcurrencyState', 'logRequestConcurrency', 'resolveNextPendingRequest',
  'waitForRequestSlot', 'releaseRequestSlot', 'runBackendRequestTask', 'runBackendRequestTasks',
]);
const output = path.join(__dirname, '.generated/script-helpers.js');
fs.mkdirSync(path.dirname(output), { recursive: true });
const build = fromBackend('esbuild').buildSync({
  absWorkingDir: root,
  metafile: true,
  stdin: {
    contents: `import ${JSON.stringify(path.join(__dirname, 'fixtures/web-apis.js'))};
      import lodash from 'lodash'; import * as yaml from 'yaml';
      import * as ageUtils from './src/utils/age';
      import {Buffer} from 'buffer'; import {Base64} from 'js-base64';
      import JSON5 from 'json5'; import dnsPacket from 'dns-packet'; import {hex_md5} from './src/vendor/md5';
      import {ipAddress, isIPv4, isIPv6, isValidUUID, getRandomPort, getPolicyDescriptor} from './src/utils';
      import YAML from './src/utils/yaml'; import {FULL} from './src/utils/logical';
      import {isMihomoConfigFile} from './src/utils/file-type';
      import {SCRIPT_RESOURCE_CACHE_KEY, DEFAULT_SCRIPT_CACHE_TTL, HEADERS_RESOURCE_CACHE_KEY, DEFAULT_HEADERS_CACHE_TTL, RESOURCE_CACHE_KEY, DEFAULT_CACHE_TTL, SETTINGS_KEY, FILES_KEY, MODULES_KEY} from './src/constants';
      Object.assign(globalThis, {lodash, yaml, Buffer, b64encode: Base64.encode, b64decode: Base64.decode});
      globalThis.ageProbeUtils = ageUtils;
      globalThis.createProbeProcessors = function($, env) {
        ${plainObject}
        ${isIP}
        ${flags}
        const SourceMMDB = (() => { ${mmdb} return MMDB; })();
        class MMDB extends SourceMMDB {
          constructor({country, asn} = {}) {
            super();
            country = country || __hostMmdbDefaults.country;
            asn = asn || __hostMmdbDefaults.asn;
            if (country) {
              const lookup = __hostMmdbOpen(country, 'Country');
              // shortcut: complete public ReaderModel objects belong to M3; this proves the source projections.
              this.countryReader = {country: ip => { const data = JSON.parse(lookup(ip));
                return {country: data.country && {isoCode: data.country.iso_code}}; }};
            }
            if (asn) {
              const lookup = __hostMmdbOpen(asn, 'ASN');
              this.asnReader = {asn: ip => { const data = JSON.parse(lookup(ip));
                return {autonomousSystemNumber: data.autonomous_system_number,
                  autonomousSystemOrganization: data.autonomous_system_organization}; }};
            }
          }
        }
        ${concurrency}
        ${flow}
        ${cache}
        const scriptResourceCache = new ResourceCache();
        const headersResourceCache = (() => { ${headersCache} return new ResourceCache(); })();
        const resourceCache = (() => { ${resourceCache} return new ResourceCache(); })();
        const HTTP = createHostHTTP, ENV = () => $.env;
        ${gist}
        // shortcut: application helpers need their M3/M4 services; never silently emulate them here.
        const unavailable = name => () => { throw new Error('M1 probe has no application helper: ' + name); };
        const ProxyUtils = {yaml: YAML, Buffer, Base64, JSON5, hex_md5, ipAddress,
          isIPv4, isIPv6, isIP, isValidUUID, getRandomPort, getFlag, getISO, removeFlag, doh, MMDB, Gist,
          age: {encrypt: ageUtils.encryptArmor, decrypt: ageUtils.decryptArmorIfPresent, derivePublicKey: ageUtils.derivePublicKey}};
        // Only the DNS parser needs these URL fields; this is not a global web-URL shim.
        const URL = function(raw) { Object.assign(this, JSON.parse(__hostDnsUrl(raw))); };
        ${dns}
        const sourceResolveDns = resolveDns;
        resolveDns = async function(options) {
          const raw = String(options.url || '').trim();
          const protocol = raw.match(/^([a-z][a-z\\d+.-]*):\\/\\//i)?.[1].toLowerCase();
          if (raw && !['auto', 'system'].includes(raw.toLowerCase()) && (!protocol || ['udp', 'tcp', 'tls'].includes(protocol))) {
            const server = parseDnsResolver(raw);
            const packet = buildDnsQuery(options);
            const response = await __hostDnsQuery(JSON.stringify(server), JSON.stringify(Array.from(packet)),
              normalizeDnsTimeout(options.timeout), options.skipCertVerify === true);
            return dnsPacket.decode(Buffer.from(JSON.parse(response)));
          }
          return sourceResolveDns(options);
        };
        ${resolvers}
        const produceArtifact = unavailable('produceArtifact');
        ProxyUtils.download = (() => {
          // Private server selection must not enable the proxy-app permanent task map or public Node APIs.
          const ENV = () => ({isNode: true});
          const getFs = () => ({readFileSync: path => Buffer.from(JSON.parse(__hostReadFile(path))).toString('utf8')});
          const getPath = () => ({isAbsolute: __hostPathIsAbsolute});
          const {AGE_SECRET_KEY, decryptArmorIfPresent, maskAgeSecretInUrl} = ageUtils;
          const tasks = new Map();
          const clashPreprocessor = {test: unavailable('download preprocess')};
          ${findByName}
          ${download}
          return (...args) => {
            // shortcut: real parser/cache eligibility needs the M3 pipeline; reject until it is integrated.
            if (args[7]) return Promise.reject(new Error('M1 probe has no application helper: download preprocess'));
            return download(...args);
          };
        })();
        ProxyUtils.downloadFile = async (url, file) => {
          $.info('Downloading file...\\nURL: ' + url + '\\nFile: ' + file);
          const result = await __hostDownloadFile(url, file);
          $.info('File downloaded from ' + url + ' to ' + file);
          return result;
        };
        const RESPONSE_TRANSFORMER = 'Response Transformer';
        ${processors}
        return {factories: {'Script Filter': ScriptFilter, 'Script Operator': ScriptOperator, 'Response Transformer': ResponseTransformer},
          ApplyProcessor, ApplyResponseTransformer, scriptResourceCache, headersResourceCache, resourceCache, DOMAIN_RESOLVERS, YAML, ProxyUtils,
          parseDnsResolver, resolveDns,
          flowUtils: {getFlowField, getFlowHeaders, parseFlowHeaders, flowTransfer, validCheck, getRmainingDays, normalizeFlowHeader}};
      };`,
    resolveDir: path.join(root, 'backend'),
  },
  bundle: true,
  alias: {'@': path.join(root, 'backend/src')},
  platform: 'browser',
  format: 'iife',
  outfile: output,
});
// Keep redistribution notices beside the generated bundle, including transitive crypto.
const notices = new Map();
for (const input of Object.keys(build.metafile.inputs).filter(input => input.includes('node_modules/'))) {
  let directory = path.dirname(path.resolve(root, input));
  let metadata;
  while (true) {
    const manifest = path.join(directory, 'package.json');
    metadata = fs.existsSync(manifest) ? JSON.parse(fs.readFileSync(manifest, 'utf8')) : null;
    if (metadata?.name && metadata.version) break;
    const parent = path.dirname(directory);
    if (parent === directory) throw new Error(`No package metadata for ${input}`);
    directory = parent;
  }
  const key = metadata.name + '@' + metadata.version;
  if (notices.has(key)) continue;
  const license = fs.readdirSync(directory).find(name => /^licen[cs]e(?:\.(?:md|txt))?$/i.test(name));
  if (!license) throw new Error(`Missing license text for ${key}`);
  notices.set(key, fs.readFileSync(path.join(directory, license), 'utf8'));
}
fs.writeFileSync(path.join(path.dirname(output), 'script-helper-licenses.txt'),
  [...notices].sort(([left], [right]) => left.localeCompare(right)).map(([name, license]) => name + '\n' + license.trim() + '\n').join('\n'));
console.log(output);
