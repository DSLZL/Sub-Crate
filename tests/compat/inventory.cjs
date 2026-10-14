// Source discovery only: every entry still needs behavioral fixtures and execution evidence.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {createRequire} = require('node:module');
const {execFileSync} = require('node:child_process');
const root = path.resolve(__dirname, '../..');
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

const frontend = path.resolve(process.argv[2] || '');
const output = path.resolve(process.argv[3] || '');
if (process.argv.length !== 4) throw new Error('Usage: inventory.cjs <pinned-frontend-root> <new-output.json>');
const frontendCommit = execFileSync('rtk', ['proxy', 'git', '-C', frontend, 'rev-parse', 'HEAD'], {encoding: 'utf8'}).trim();
assert.equal(frontendCommit, '4bd7b0acbe65466b933ccea14fc61dbebe036ead');
const report = {backend_commit: '9cc6b5fac966993364ae33da8797f446bbf7f571', frontend_commit: frontendCommit,
  complete_behavior_inventory: false, routes: [], proxy_formats: [], registries: {}, frontend_requests: [], frontend_url_literals: [], parse_gaps: [],
  unresolved_behavior: ['Route parameter/query/coercion/error/state branches', 'Dynamic frontend type values and direct URL builders',
    'Vue template bindings and caller workflow mapping', 'Protocol/processor option combinations', 'Runtime/deployment/integration modes']};
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
  report.registries[relative] = [];
  traverse(parse(source), {ExportDefaultDeclaration(p) {
    const items = p.node.declaration.properties || p.node.declaration.elements;
    if (!items) { report.registries[relative].push({expression: source.slice(p.node.declaration.start, p.node.declaration.end), unresolved: true}); return; }
    for (const item of items) {
      report.registries[relative].push({key: item.key?.name || item.key?.value || null,
        expression: source.slice((item.value || item).start, (item.value || item).end), source: `${relative}:${item.loc.start.line}`, execution: 'not_mapped'});
    }
  }});
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
fs.mkdirSync(path.dirname(output), {recursive: true});
fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n', {flag: 'wx'});
console.log(JSON.stringify({routes: report.routes.length, proxy_aliases: report.proxy_formats.length,
  frontend_requests: report.frontend_requests.length, frontend_url_literals: report.frontend_url_literals.length,
  parse_gaps: report.parse_gaps.length, complete_behavior_inventory: false, output}));
if (report.parse_gaps.length) process.exitCode = 1;
