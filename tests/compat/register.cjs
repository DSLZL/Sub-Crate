// The pinned Babel alias plugin splits filenames at '/', including Windows filenames.
// Resolve the same aliases using native absolute paths in the development-only loader.
const assert = require('node:assert/strict');
const path = require('node:path');
const { createRequire } = require('node:module');
const fromBackend = createRequire(path.resolve(__dirname, '../../backend/package.json'));
const sourceRoot = path.resolve(__dirname, '../../backend/src');
function resolveAlias(value) {
  return value.startsWith('@/') ? path.join(sourceRoot, value.slice(2)).replace(/\\/g, '/') : value;
}
const aliasPlugin = () => ({ visitor: {
  ImportDeclaration({node}) { node.source.value = resolveAlias(node.source.value); },
  ExportNamedDeclaration({node}) { if (node.source) node.source.value = resolveAlias(node.source.value); },
  ExportAllDeclaration({node}) { node.source.value = resolveAlias(node.source.value); },
  CallExpression({node}) {
    if (node.callee.name !== 'require') return;
    let arg = node.arguments[0];
    while (arg?.type === 'BinaryExpression') arg = arg.left;
    if (arg?.type === 'StringLiteral') arg.value = resolveAlias(arg.value);
  },
}});
assert.equal(resolveAlias('@/utils/yaml'), path.join(sourceRoot, 'utils/yaml').replace(/\\/g, '/'));
fromBackend('@babel/register')({
  babelrc: false, configFile: false, cache: false,
  presets: [fromBackend.resolve('@babel/preset-env')], plugins: [aliasPlugin],
});
