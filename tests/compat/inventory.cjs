// Source discovery only: every entry still needs behavioral fixtures and execution evidence.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {createRequire} = require('node:module');
const {execFileSync, spawnSync} = require('node:child_process');
const {createHash} = require('node:crypto');
const root = path.resolve(__dirname, '../..');
if (process.env.GITHUB_ACTIONS !== 'true') throw new Error('Generate compatibility inventory in GitHub Actions only');
const hasRtk = spawnSync('rtk', ['--version'], {stdio: 'ignore'}).status === 0;
if (!hasRtk) console.error('RTK unavailable on this runner; use direct Git for source provenance');
const git = (...args) => execFileSync(hasRtk ? 'rtk' : 'git', [...(hasRtk ? ['proxy', 'git'] : []), ...args], {encoding: 'utf8'});
const fromBackend = createRequire(path.join(root, 'backend/package.json'));
const {parseSync, traverse} = fromBackend('@babel/core');
const methods = new Set(['get', 'post', 'put', 'patch', 'delete', 'options', 'head', 'all']);

function files(directory, extensions) {
  return fs.readdirSync(directory, {withFileTypes: true}).flatMap(entry => {
    const filename = path.join(directory, entry.name);
    return entry.isDirectory() ? files(filename, extensions) : extensions.includes(path.extname(filename)) ? [filename] : [];
  }).sort();
}
function parse(source, typescript = false) {
  return parseSync(source, {babelrc: false, configFile: false, sourceType: 'unambiguous',
    parserOpts: {plugins: typescript ? ['typescript'] : []}});
}
function appRoute(node) {
  if (node?.type !== 'CallExpression' || node.callee.type !== 'MemberExpression') return null;
  const receiver = node.callee.object;
  if (receiver.type === 'Identifier' && receiver.name === '$app') {
    return node.arguments[0]?.type === 'StringLiteral' ? node.arguments[0].value : null;
  }
  return appRoute(receiver);
}
function findRoutes(source, filename) {
  const result = [];
  traverse(parse(source), {CallExpression(p) {
    const node = p.node;
    const method = node.callee.type === 'MemberExpression' && node.callee.property.name;
    const route = appRoute(node);
    if (!methods.has(method) || !route) return;
    const direct = node.callee.object.type === 'Identifier';
    const handler = node.arguments[direct ? 1 : 0];
    const conditions = p.getAncestry().filter(parent => parent.isIfStatement()).map(parent => source.slice(parent.node.test.start, parent.node.test.end));
    result.push({method: method.toUpperCase(), path: route, source: `${filename}:${node.loc.start.line}`,
      handler: handler?.type === 'Identifier' ? handler.name : 'inline', enclosing_conditions: conditions, execution: 'not_mapped'});
  }});
  return result;
}
assert.deepEqual(findRoutes("$app.route('/api/sub/:name').get(getSub).patch(updateSub); $app.post('/api/storage', restore);", 'self-check')
  .map(route => `${route.method} ${route.path}`), ['PATCH /api/sub/:name', 'GET /api/sub/:name', 'POST /api/storage']);
assert.equal(findRoutes("res.get('header'); obj.post('other');", 'self-check').length, 0);

function findRegistry(source, filename) {
  const ast = parse(source);
  const constants = new Map();
  for (const statement of ast.program.body) {
    const declaration = statement.type === 'ExportNamedDeclaration' ? statement.declaration : statement;
    if (declaration?.type !== 'VariableDeclaration' || declaration.kind !== 'const') continue;
    for (const item of declaration.declarations) {
      if (item.id.type === 'Identifier' && item.init?.type === 'StringLiteral') constants.set(item.id.name, item.init.value);
    }
  }
  const result = [];
  traverse(ast, {ExportDefaultDeclaration(p) {
    const items = p.node.declaration.properties || p.node.declaration.elements;
    if (!items) { result.push({expression: source.slice(p.node.declaration.start, p.node.declaration.end), unresolved: true}); return; }
    for (const item of items) {
      let key;
      if (item.type === 'ObjectProperty') {
        if (item.key.type === 'StringLiteral') key = item.key.value;
        else key = item.computed ? constants.get(item.key.name) : item.key.name;
      } else if (item.type === 'CallExpression' && item.callee.type === 'Identifier') key = item.callee.name;
      result.push({key: key ?? null, kind: item.type === 'ObjectProperty' ? 'named' : 'factory',
        expression: source.slice((item.value || item).start, (item.value || item).end), source: `${filename}:${item.loc.start.line}`,
        execution: 'not_mapped', ...(key === undefined ? {unresolved: true} : {})});
    }
  }});
  return result;
}
assert.deepEqual(findRegistry("export const TYPE = 'Resolved'; function inner() { const TYPE = 'Wrong'; } export default {[TYPE]: Factory, 'Alias': Factory};", 'self-check')
  .map(item => item.key), ['Resolved', 'Alias']);
assert.deepEqual(findRegistry('export default [First(), Second()];', 'self-check').map(item => item.key), ['First', 'Second']);
assert.equal(findRegistry('export default {[MISSING]: Factory};', 'self-check')[0].unresolved, true);

const frontend = path.resolve(process.argv[2] || '');
const output = path.resolve(process.argv[3] || '');
if (process.argv.length !== 4) throw new Error('Usage: inventory.cjs <pinned-frontend-root> <new-output.json>');
const frontendCommit = git('-C', frontend, 'rev-parse', 'HEAD').trim();
assert.equal(frontendCommit, '4bd7b0acbe65466b933ccea14fc61dbebe036ead');
git('-C', frontend, 'diff', '--exit-code', 'HEAD');
const backendBaseCommit = '9cc6b5fac966993364ae33da8797f446bbf7f571';
const backendPatch = git('-C', root, 'diff', '--no-ext-diff', '--binary', '--no-color', '--full-index', '--unified=0', backendBaseCommit, '--',
  'backend', 'scripts/ip-flag-node.js');
assert.equal(backendPatch, fs.readFileSync(path.join(__dirname, 'approved-reference-fixes.patch'), 'utf8').replace(/\r\n/g, '\n'),
  'Backend source differs from the frozen base plus approved RF01/RF02 patch');
const backendCommit = git('-C', root, 'rev-parse', 'HEAD').trim();
const frontendMap = JSON.parse(fs.readFileSync(path.join(__dirname, 'frontend-contract-map.json'), 'utf8'));
assert.equal(frontendMap.frontend_commit, frontendCommit);
const frontendCases = new Set([...fs.readFileSync(path.join(__dirname, 'frontend.py'), 'utf8')
  .matchAll(/"id": "(FE\d+-[^"]+)"/g)].map(match => match[1]));
assert.ok(frontendCases.size, 'No executable frontend workflow IDs found');
const report = {backend_commit: backendCommit, backend_base_commit: backendBaseCommit, frontend_commit: frontendCommit,
  approved_reference_patch_sha256: createHash('sha256').update(backendPatch).digest('hex'),
  complete_behavior_inventory: false, routes: [], proxy_formats: [], registries: {}, frontend_requests: [], frontend_url_literals: [],
  executable_frontend_workflow_ids: [...frontendCases], parse_gaps: [],
  configuration_identifiers: [], standalone_web_global_references: [],
  unresolved_behavior: ['Route parameter/query/coercion/error/state branches', 'Dynamic frontend type values and direct URL builders',
    'Vue template bindings and caller workflow mapping', 'Protocol/processor option combinations', 'Runtime/deployment/integration modes']};
const configuration = new Map();
const webGlobals = new Set(['fetch', 'FormData', 'Blob', 'File', 'AbortController', 'ReadableStream', 'WebSocket']);
function webReferences(source, relative) {
  const result = [];
  traverse(parse(source), {ReferencedIdentifier(p) {
    if (webGlobals.has(p.node.name) && !p.scope.hasBinding(p.node.name)) {
      result.push({name: p.node.name, source: `${relative}:${p.node.loc.start.line}`});
    }
  }});
  return result;
}
assert.deepEqual(webReferences("fetch('/'); function local(FormData) { return new FormData(); } new Blob([]);", 'self-check')
  .map(item => item.name), ['fetch', 'Blob']);
for (const filename of files(path.join(root, 'backend/src'), ['.js']).filter(name => !name.includes(path.sep + 'test' + path.sep))) {
  const source = fs.readFileSync(filename, 'utf8');
  const relative = path.relative(root, filename).split(path.sep).join('/');
  for (const match of source.matchAll(/\bSUB_STORE_[A-Z0-9_]+\b/g)) {
    const anchor = `${relative}:${source.slice(0, match.index).split('\n').length}`;
    if (!configuration.has(match[0])) configuration.set(match[0], new Set());
    configuration.get(match[0]).add(anchor);
  }
  report.standalone_web_global_references.push(...webReferences(source, relative));
}
report.configuration_identifiers = [...configuration].sort(([a], [b]) => a.localeCompare(b))
  .map(([name, sources]) => ({name, sources: [...sources]}));
assert.equal(configuration.size, 32, 'Frozen standalone SUB_STORE identifier inventory changed');
for (const filename of files(path.join(root, 'backend/src/restful'), ['.js'])) {
  report.routes.push(...findRoutes(fs.readFileSync(filename, 'utf8'), path.relative(root, filename).replace(/\\/g, '/')));
}
const producerFile = path.join(root, 'backend/src/core/proxy-utils/producers/index.js');
traverse(parse(fs.readFileSync(producerFile, 'utf8')), {ExportDefaultDeclaration(p) {
  for (const property of p.node.declaration.properties) {
    report.proxy_formats.push({alias: property.key.name || property.key.value, family: property.value.callee.name,
      source: `backend/src/core/proxy-utils/producers/index.js:${property.loc.start.line}`, execution: 'not_mapped'});
  }
}});
for (const relative of ['backend/src/core/proxy-utils/processors/index.js', 'backend/src/core/rule-utils/producers.js',
  'backend/src/core/proxy-utils/parsers/index.js', 'backend/src/core/proxy-utils/preprocessors/index.js']) {
  const source = fs.readFileSync(path.join(root, relative), 'utf8');
  report.registries[relative] = findRegistry(source, relative);
  for (const item of report.registries[relative]) {
    if (item.unresolved) report.parse_gaps.push({file: relative, error: `Unresolved registry entry: ${item.expression}`});
  }
}
for (const filename of files(path.join(frontend, 'src'), ['.ts', '.js', '.vue'])) {
  const fullSource = fs.readFileSync(filename, 'utf8');
  const blocks = filename.endsWith('.vue') ? [...fullSource.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)]
    .map(match => ({source: match[1], lineOffset: fullSource.slice(0, match.index + match[0].indexOf('>') + 1).split('\n').length - 1}))
    : [{source: fullSource, lineOffset: 0}];
  for (const {source, lineOffset} of blocks) {
    try {
      const ast = parse(source, !filename.endsWith('.js'));
      const anchor = node => `frontend/${path.relative(frontend, filename).replace(/\\/g, '/')}:${node.loc.start.line + lineOffset}`;
      traverse(ast, {
        ObjectExpression(p) {
          const properties = p.node.properties.filter(node => node.type === 'ObjectProperty');
          const url = properties.find(node => (node.key.name || node.key.value) === 'url');
          const method = properties.find(node => (node.key.name || node.key.value) === 'method');
          if (!url || !method) return;
          const owner = p.findParent(parent => parent.isObjectProperty() || parent.isFunctionDeclaration());
          report.frontend_requests.push({method: method.value.value?.toUpperCase() || source.slice(method.value.start, method.value.end),
            url_expression: source.slice(url.value.start, url.value.end), source: anchor(url),
            caller: owner?.node.key?.name || owner?.node.id?.name || 'inline',
            dynamic_values_resolved: url.value.type === 'StringLiteral' || (url.value.type === 'TemplateLiteral' && !url.value.expressions.length),
            fixture: null, execution: 'not_mapped'});
        },
        'StringLiteral|TemplateLiteral'(p) {
          const expression = source.slice(p.node.start, p.node.end);
          if (/\/(api|download|share)\//.test(expression)) report.frontend_url_literals.push({source: anchor(p.node), expression, execution: 'not_mapped'});
        },
      });
    } catch (error) { report.parse_gaps.push({file: path.relative(frontend, filename).replace(/\\/g, '/'), error: error.message}); }
  }
}
const mapped = new Set();
const fixtures = JSON.parse(fs.readFileSync(path.join(__dirname, 'fixtures/http-cases.json'), 'utf8'));
const fixtureIds = new Set(fixtures.map(item => item.id));
for (const format of report.proxy_formats) {
  const id = `P01-output-${format.alias}`;
  const fixture = fixtures.find(item => item.id === id);
  assert.ok(fixture, `Missing proxy output alias fixture: ${format.alias}`);
  assert.ok(fixture.steps.some(step => step.path === '/api/proxy/parse' && step.body?.client === format.alias && step.expect.status === 200), id);
  assert.ok(fixture.steps.some(step => step.path.startsWith(`/download/unused/${format.alias}?`) && step.expect.status === 200), id);
  Object.assign(format, {fixture: id, coverage: 'one SS input through parse and download routes', execution: 'not_evaluated'});
}
for (const producer of report.registries['backend/src/core/rule-utils/producers.js']) {
  const ids = [`R01-output-${producer.key}`, `R02-unsupported-${producer.key}`];
  for (const id of ids) assert.ok(fixtureIds.has(id), `Missing rule producer fixture: ${id}`);
  Object.assign(producer, {fixtures: ids, coverage: 'base and unsupported rules', execution: 'not_evaluated'});
}
for (const request of report.frontend_requests) {
  const api = request.source.match(/^frontend\/src\/api\/([^/]+)\//)?.[1];
  const key = `${api}.${request.caller}`;
  const mapping = frontendMap.requests[key];
  assert.ok(mapping, `Unmapped frontend API request: ${key} (${request.source})`);
  assert.ok(!mapped.has(key), `Duplicate frontend mapping: ${key}`);
  mapped.add(key);
  assert.equal(mapping.method, request.method, key);
  for (const route of mapping.paths) assert.ok(report.routes.some(item => item.path === route && [request.method, 'ALL'].includes(item.method)), `Missing backend route for ${key}: ${route}`);
  for (const id of mapping.http_cases) assert.ok(fixtureIds.has(id), `Missing HTTP fixture for ${key}: ${id}`);
  for (const id of mapping.ui_cases) assert.ok(frontendCases.has(id), `Missing UI workflow for ${key}: ${id}`);
  Object.assign(request, mapping, {mapping_status: 'source_mapped', execution: 'not_evaluated'});
}
assert.deepEqual([...mapped].sort(), Object.keys(frontendMap.requests).sort(), 'Stale frontend mapping entries');
report.frontend_dynamic_domains = frontendMap.dynamic_domains;
report.frontend_direct_builders = frontendMap.direct_builders;
for (const builder of frontendMap.direct_builders) {
  for (const id of builder.ui_cases || []) assert.ok(frontendCases.has(id), `Missing UI workflow for ${builder.source}: ${id}`);
}
fs.mkdirSync(path.dirname(output), {recursive: true});
fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n', {flag: 'wx'});
console.log(JSON.stringify({routes: report.routes.length, proxy_aliases: report.proxy_formats.length,
  frontend_requests: report.frontend_requests.length, frontend_url_literals: report.frontend_url_literals.length,
  frontend_workflows: frontendCases.size,
  registries: Object.fromEntries(Object.entries(report.registries).map(([source, entries]) => [source, entries.length])),
  parse_gaps: report.parse_gaps.length, complete_behavior_inventory: false, output}));
if (report.parse_gaps.length) process.exitCode = 1;
