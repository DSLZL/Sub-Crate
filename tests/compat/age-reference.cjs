// Fresh Node oracle using the actual frozen exports and processor injection.
const fs = require('node:fs');
const path = require('node:path');
const state = process.env.SUB_STORE_COMPAT_STATE_DIR;
const ageInput = JSON.parse(fs.readFileSync(path.join(state, 'age-input.json'), 'utf8'));
const ageProbeUtils = require(path.resolve(__dirname, '../../backend/src/utils/age.js'));
const source = require(path.resolve(__dirname, '../../backend/src/core/proxy-utils/processors/index.js'));
const processorHost = {factories: source.default, ApplyProcessor: source.ApplyProcessor,
  ProxyUtils: require(path.resolve(__dirname, '../../backend/src/core/proxy-utils/index.js')).ProxyUtils};
const probeKind = 'node';
const probeAgeProgress = (id, event) => fs.writeSync(2, `COMPAT_AGE ${JSON.stringify({id, event})}\n`);
eval(fs.readFileSync(path.join(__dirname, 'fixtures/script-age-contracts.js'), 'utf8')).then(result => {
  fs.writeFileSync(path.join(state, 'age-output.json'), JSON.stringify(result.exchange) + '\n');
  delete result.exchange;
  fs.writeSync(1, `COMPAT_AGE_RESULT ${JSON.stringify(result)}\n`);
}, error => { fs.writeSync(2, `${error.stack}\n`); process.exitCode = 1; });
