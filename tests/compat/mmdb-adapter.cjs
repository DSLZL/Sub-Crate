// Build-only native I/O seam for the pinned generic SDK reader and models.
const crypto = require('node:crypto');
if (process.env.GITHUB_ACTIONS !== 'true') throw new Error('Adapt MMDB utilities in GitHub Actions only');
module.exports = function adapt(bytes) {
  const original = crypto.createHash('sha256').update(bytes).digest('hex');
  if (original !== '1135a1227bc055fc96104316ad917e4f4da03df2b78a5ab8686c4a41296fe3b5') {
    throw new Error('Pinned MMDB reader source changed');
  }
  let source = bytes.toString('utf8').replace(/\r\n/g, '\n');
  for (const [before, after] of [
    ['const ip6addr = require("ip6addr");\n', ''],
    ['const mmdb = require("maxmind");\n', ''],
    ['if (!mmdb.validate(ipAddress)) {', 'if (!this.mmdbReader.validate(ipAddress)) {'],
    ['const [record, prefixLength] = this.mmdbReader.getWithPrefixLength(ipAddress);',
     'const [record, network] = this.mmdbReader.getWithNetwork(ipAddress);'],
    ['return [record, ip6addr.createCIDR(ipAddress, prefixLength).toString()];', 'return [record, network];'],
  ]) {
    if (source.split(before).length !== 2) throw new Error('MMDB adapter source boundary changed');
    source = source.replace(before, after);
  }
  if (/node:|require\(["'](?:maxmind|ip6addr)["']\)/.test(source)) throw new Error('MMDB reader retained a Node runtime dependency');
  return {source, provenance: {adapted_from: '@maxmind/geoip2-node@5.0.0/dist/src/readerModel.js',
                              original_sha256: original, native_seam: 'validate and getWithNetwork', keep_names: true}};
};
