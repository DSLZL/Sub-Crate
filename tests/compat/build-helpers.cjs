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
    const matches = nodes.filter(node => ['FunctionDeclaration', 'ClassDeclaration'].includes(node.type) && node.id.name === name);
    if (matches.length !== 1) throw new Error(`Expected one ${name} in ${relative}`);
    return source.slice(matches[0].start, matches[0].end);
  }).join('\n');
}
const processors = declarations('backend/src/core/proxy-utils/processors/index.js', [
  'trimWrap', 'deepMerge', 'ScriptOperator', 'ScriptFilter', 'ResponseTransformer',
  'ApplyResponseTransformer', 'ApplyFilter', 'ApplyOperator', 'ApplyProcessor', 'clone', 'createDynamicFunction',
]);
const cache = declarations('backend/src/utils/script-resource-cache.js', ['ResourceCache', 'normalizeTTL', 'getTTL']);
const plainObject = declarations('backend/src/utils/index.js', ['isPlainObject']);
const output = path.join(__dirname, '.generated/script-helpers.js');
fs.mkdirSync(path.dirname(output), { recursive: true });
fromBackend('esbuild').buildSync({
  stdin: {
    contents: `import lodash from 'lodash'; import * as yaml from 'yaml';
      import {Buffer} from 'buffer'; import {Base64} from 'js-base64';
      import YAML from './src/utils/yaml'; import {FULL} from './src/utils/logical';
      import {isMihomoConfigFile} from './src/utils/file-type';
      import {SCRIPT_RESOURCE_CACHE_KEY, DEFAULT_SCRIPT_CACHE_TTL, SETTINGS_KEY} from './src/constants';
      Object.assign(globalThis, {lodash, yaml, Buffer, b64encode: Base64.encode, b64decode: Base64.decode});
      globalThis.createProbeProcessors = function($, env) {
        ${plainObject}
        ${cache}
        const scriptResourceCache = new ResourceCache();
        // shortcut: application helpers need their M3/M4 services; never silently emulate them here.
        const unavailable = name => () => { throw new Error('M1 probe has no application helper: ' + name); };
        const ProxyUtils = {yaml: YAML, Buffer, Base64};
        const DOMAIN_RESOLVERS = {};
        const [getFlowField, getFlowHeaders, parseFlowHeaders, flowTransfer, validCheck, getRmainingDays, normalizeFlowHeader, produceArtifact] =
          ['getFlowField', 'getFlowHeaders', 'parseFlowHeaders', 'flowTransfer', 'validCheck', 'getRmainingDays', 'normalizeFlowHeader', 'produceArtifact'].map(unavailable);
        const RESPONSE_TRANSFORMER = 'Response Transformer';
        ${processors}
        return {factories: {'Script Filter': ScriptFilter, 'Script Operator': ScriptOperator, 'Response Transformer': ResponseTransformer},
          ApplyProcessor, ApplyResponseTransformer, scriptResourceCache, YAML};
      };`,
    resolveDir: path.join(root, 'backend'),
  },
  bundle: true,
  platform: 'browser',
  format: 'iife',
  outfile: output,
});
console.log(output);
