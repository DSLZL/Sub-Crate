// Development oracle: execute the actual source OpenAPI, never a copied host model.
const fs = require('node:fs');
const path = require('node:path');
const {OpenAPI} = require(path.resolve(__dirname, '../../backend/src/vendor/open-api.js'));
const $substore = new OpenAPI('sub-store-probe');
const probeUpstream = process.env.SUB_STORE_COMPAT_UPSTREAM;
const script = process.argv.includes('--resume')
  ? 'Promise.resolve({storage: {cache: $substore.cache, root: $substore.root}})'
  : fs.readFileSync(path.join(__dirname, 'fixtures/script-host-contracts.js'), 'utf8');
eval(script).then(result => {
  process.stdout.write(`COMPAT_HOST_RESULT ${JSON.stringify(result)}\n`);
}, error => {
  fs.writeSync(2, `${error.stack}\n`);
  process.exitCode = 1;
});
