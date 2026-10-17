// Fresh development oracle: unchanged download/ProxyUtils/application exports.
const fs = require('node:fs');
const path = require('node:path');
const state = process.env.SUB_STORE_COMPAT_STATE_DIR;
const fixture = name => fs.readFileSync(path.join(__dirname, 'fixtures', name), 'utf8');
async function main() {
  if (process.argv.includes('--prepare')) {
    const age = require(path.resolve(__dirname, '../../backend/src/utils/age.js'));
    const keys = [];
    for (const type of ['x25519', 'mlkem768-x25519']) {
      const key = await age.generateKeyPair(type);
      keys.push({...key, armor: await age.encryptArmor('下载🙂\r\n\0尾\n', key['age-public-key'])});
    }
    fs.writeFileSync(path.join(state, 'download-input.json'), JSON.stringify({keys}) + '\n');
    fs.writeSync(1, 'COMPAT_DOWNLOAD_PREPARED 2\n');
    return;
  }
  eval(fixture('script-processor-clock.js'));
  const $substore = require(path.resolve(__dirname, '../../backend/src/core/app.js')).default;
  const source = require(path.resolve(__dirname, '../../backend/src/core/proxy-utils/processors/index.js'));
  const processorHost = {
    factories: source.default, ApplyProcessor: source.ApplyProcessor,
    ProxyUtils: require(path.resolve(__dirname, '../../backend/src/core/proxy-utils/index.js')).ProxyUtils,
    resourceCache: require(path.resolve(__dirname, '../../backend/src/utils/resource-cache.js')).default,
    headersResourceCache: require(path.resolve(__dirname, '../../backend/src/utils/headers-resource-cache.js')).default,
  };
  const processorDiagnostics = [];
  for (const level of ['log', 'info', 'warn', 'error']) $substore[level] = message => processorDiagnostics.push(String(message));
  const probeKind = 'node', probeMmdbRoot = state;
  const probeUpstream = process.env.SUB_STORE_COMPAT_UPSTREAM;
  const probeReadFile = file => fs.readFileSync(file);
  const probeDownloadInput = JSON.parse(fs.readFileSync(path.join(state, 'download-input.json'), 'utf8'));
  const probeDownloadProgress = (id, event) => fs.writeSync(2, `COMPAT_DOWNLOAD ${JSON.stringify({id, event})}\n`);
  const result = await eval(fixture(process.argv.includes('--resume') ? 'script-download-resume.js' : 'script-download-contracts.js'));
  process.stdout.write(`COMPAT_DOWNLOAD_RESULT ${JSON.stringify(result)}\n`);
}
main().catch(error => { fs.writeSync(2, `${error.stack}\n`); process.exitCode = 1; });
