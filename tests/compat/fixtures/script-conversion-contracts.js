globalThis.runConversionContracts = async function(check, host, operator, assert, equal) {
  const utils = host.ProxyUtils;
  const copy = value => JSON.parse(JSON.stringify(value));
  const uuid = '11111111-1111-4111-8111-111111111111';
  const ss = {type: 'ss', 'skip-cert-verify': false, udp: true, server: 'ss.example.com', port: 8388, cipher: 'aes-128-gcm', password: 'secret', name: '节点🙂'};
  const uri = 'ss://' + utils.Base64.encode('aes-128-gcm:secret') + '@ss.example.com:8388#' + encodeURIComponent(ss.name);
  function subset(actual, expected) {
    for (const [key, value] of Object.entries(expected)) equal(actual[key], value, 'conversion field ' + key);
    return actual;
  }
  function one(raw, expected) {
    const result = utils.parse(raw);
    equal(result.length, 1, 'one parsed proxy');
    if (expected) subset(result[0], expected);
    return result[0];
  }
  function rejects(callback, type, message) {
    let caught = false;
    try { callback(); } catch (error) {
      caught = true;
      assert(error instanceof type, 'native conversion error prototype');
      if (message) equal(error.message, message, 'conversion error message');
    }
    assert(caught, 'conversion must reject');
    return true;
  }
  const aliases = [
    ['qx', 'QX', 'QuantumultX'], ['surge', 'Surge'], ['SurgeMac'], ['Loon'], ['Clash'],
    ['meta', 'clashmeta', 'clash.meta', 'Clash.Meta', 'ClashMeta', 'mihomo', 'Mihomo'],
    ['uri', 'URI'], ['v2', 'v2ray', 'V2Ray'], ['json', 'JSON'], ['stash', 'Stash'],
    ['shadowrocket', 'Shadowrocket', 'ShadowRocket'], ['surfboard', 'Surfboard'],
    ['singbox', 'sing-box'], ['egern', 'Egern'],
  ];
  await check('PC89-conversion-mixed-lines-and-parser-reuse', () => {
    const result = utils.parse('  ' + uri + '\r\n' + uri + '\ninvalid://line\n\nhttps://alice:pa%24%24@https.example.com#HTTPS%20Outer#Remark\n' + uri);
    equal(result.length, 4, 'bad lines skipped and last-parser fallback recovers');
    equal(result[0], ss, 'SIP002 canonical fields');
    equal(result[1], ss, 'last parser reused');
    subset(result[2], {type: 'http', name: 'HTTPS Outer#Remark', server: 'https.example.com', port: 443, tls: true, username: 'alice', password: 'pa$$'});
    equal(result[3], ss, 'parser switches back');
    return result;
  });
  await check('PC90-conversion-preprocessors-and-short-id', () => {
    equal(utils.parse('<!DOCTYPE html><html>error</html>'), [], 'HTML discarded');
    equal(utils.parse(utils.Base64.encode(uri)), [ss], 'base64 subscription');
    const ssd = 'ssd://' + utils.Base64.encode(JSON.stringify({port: 8388, encryption: ss.cipher, password: ss.password, servers: [{server: ss.server, remarks: ss.name}]}));
    equal(utils.parse(ssd), [ss], 'SSD defaults');
    const config = 'proxies:\n  - {name: reality, type: vless, server: v.example, port: 443, uuid: ' + uuid + ', tls: true, reality-opts: {public-key: key, short-id: 0088}}\n';
    const reality = one(config, {type: 'vless', name: 'reality'});
    equal(reality['reality-opts']['short-id'], '0088', 'short-id leading zeros preserved');
    const full = one('[Proxy]\n' + 'full = ss, ss.example.com, 8388, encrypt-method=aes-128-gcm, password=secret\n[Rule]\nFINAL,DIRECT', {type: 'ss', name: 'full'});
    return {base64: ss, ssd: ss, reality, full};
  });
  await check('PC91-conversion-peggy-client-grammars', () => {
    const lines = [
      ['Surge = ss, ss.example.com, 8388, encrypt-method=aes-128-gcm, password=secret, udp-relay=true', 'Surge'],
      ['Loon=shadowsocks,ss.example.com,8388,aes-128-gcm,"secret",udp=true', 'Loon'],
      ['shadowsocks=ss.example.com:8388,method=aes-128-gcm,password=secret,udp-relay=true,tag=QX', 'QX'],
    ];
    return lines.map(([line, name]) => one(line, {type: 'ss', name, server: ss.server, port: ss.port, cipher: ss.cipher, password: ss.password, udp: true}));
  });
  await check('PC92-conversion-uri-families-and-transports', () => {
    const cases = [
      ['socks://' + encodeURIComponent(utils.Base64.encode('bob:secret')) + '@socks.example.com:1080#SOCKS', {type: 'socks5', username: 'bob', password: 'secret', port: 1080}],
      ['vmess://' + utils.Base64.encode(JSON.stringify({v: '2', ps: 'VMess🙂', add: 'vm.example', port: '443', id: uuid, aid: '0', scy: 'auto', net: 'ws', host: 'cdn.example', path: '/ws', tls: 'tls'})), {type: 'vmess', name: 'VMess🙂', uuid, port: 443, network: 'ws', tls: true}],
      ['vless://' + uuid + '@v.example:443?type=ws&security=tls&host=cdn.example&path=%2Fws#VLESS', {type: 'vless', name: 'VLESS', uuid, network: 'ws', tls: true}],
      ['trojan://secret@tr.example:443?allowInsecure=1#Trojan', {type: 'trojan', password: 'secret', tls: true}],
      ['hysteria2://secret@hy.example:443?obfs=salamander&obfs-password=mask#HY2', {type: 'hysteria2', password: 'secret', obfs: 'salamander', 'obfs-password': 'mask'}],
      ['tuic://' + uuid + ':pass%3Aword@tuic.example.com?alpn=h3,hq-29&allow-insecure=1&congestion-control=bbr#TUIC', {type: 'tuic', uuid, password: 'pass:word', alpn: ['h3', 'hq-29'], 'congestion-controller': 'bbr'}],
      ['anytls://secret@any.example:443#AnyTLS', {type: 'anytls', password: 'secret', tls: true}],
    ];
    return cases.map(([raw, expected]) => one(raw, expected));
  });
  await check('PC93-conversion-normalization-and-unknown-fields', () => {
    const raw = {type: 'vmess', name: 0, server: ' [2001:db8::1] ', port: '443', uuid, cipher: 'AUTO', password: 1234567890123456,
      udp: 'OFF', tls: true, network: 'ws', 'WS-OPTS': {path: ' ws ', headers: {Host: 'cdn.example'}, unknown: {flag: false}},
      'ws-opts': {path: 0, headers: {Host: 'selected.example'}}, unknown: {nil: null, zero: 0, flag: false}};
    const result = one(JSON.stringify(raw), {name: '0', server: '2001:db8::1', port: 443, cipher: 'auto', password: '1234567890123456', udp: false, sni: 'selected.example', unknown: raw.unknown});
    assert(!Object.hasOwn(result, 'WS-OPTS'), 'uppercase opts alias removed without replacing existing lowercase');
    equal(result['ws-opts'].path, '/0', 'numeric transport path');
    const normalized = one(JSON.stringify({type: 'vless', name: 'opts', server: 'v.example', port: 443, uuid, network: 'http', 'HTTP-OPTS': {PATH: [' ', 'api'], HEADERS: {Host: 'cdn.example'}}}));
    equal(normalized['http-opts'].path, ['/', '/api'], 'nested opts and path array');
    return {result, normalized};
  });
  await check('PC94-conversion-hysteria-and-wireguard-normalization', () => {
    const hy = one(JSON.stringify({type: 'hysteria2', name: 'hy', server: 'hy.example', port: 443, password: 'p', obfs: 'mask', ports: '1000/1001', 'hop-interval': '10 - 20'}));
    subset(hy, {obfs: 'salamander', 'obfs-password': 'mask', ports: '1000,1001', 'hop-interval': 10, 'hop-interval-max': 20});
    const wg = one(JSON.stringify({type: 'wireguard', name: 'wg', server: 'wg.example', port: 51820, ip: '10.0.0.2/32', ipv6: 'fd00::2/128', 'private-key': 'private', 'public-key': 'public', mtu: 0}));
    subset(wg, {type: 'wireguard', mtu: 0});
    const invalid = one(JSON.stringify({type: 'hysteria2', name: 'bad-hop', server: 'hy.example', port: 443, password: 'p', 'hop-interval': '20-10'}));
    assert(!Object.hasOwn(invalid, 'hop-interval') && !Object.hasOwn(invalid, 'hop-interval-max'), 'invalid range removed');
    return {hy, wg, invalid};
  });
  await check('PC95-conversion-certificate-fingerprint', () => {
    const result = one(JSON.stringify({type: 'trojan', name: 'ca', server: 'tr.example', port: 443, password: 'p', tls: true, ca_str: '-----BEGIN CERTIFICATE-----\nAAECAw==\n-----END CERTIFICATE-----'}));
    equal(result['tls-fingerprint'], '05:4E:DE:C1:D0:21:1F:62:4F:ED:0C:BC:A9:D4:F9:40:0B:0E:49:1C:43:74:2A:F2:C5:B0:AB:EB:F0:C9:90:D8', 'SHA256 of exact DER bytes 0,1,2,3');
    assert(Object.hasOwn(result, 'ca-str') && !Object.hasOwn(result, 'ca_str'), 'CA alias normalized');
    return result;
  });
  await check('PC96-conversion-all-33-aliases-and-representations', () => {
    equal(aliases.flat().length, 33, 'source alias denominator');
    const outputs = {};
    for (const family of aliases) {
      const external = utils.produce([copy(ss)], family[0], 'external');
      const internal = utils.produce([copy(ss)], family[0], 'internal');
      assert(typeof external === 'string' && external.length > 0, 'nonempty external ' + family[0]);
      if (family[0] === 'v2') equal(internal, external, 'V2Ray always emits base64 text including internal mode');
      else assert(Array.isArray(internal) && internal.length === 1, 'internal array ' + family[0]);
      for (const alias of family) {
        equal(utils.produce([copy(ss)], alias, 'external'), external, 'external alias ' + alias);
        equal(utils.produce([copy(ss)], alias, 'internal'), internal, 'internal alias ' + alias);
        equal(utils.produce([copy(ss)], alias), external, 'default representation ' + alias);
      }
      outputs[family[0]] = {aliases: family, external, internal};
    }
    return outputs;
  });
  await check('PC97-conversion-json-identity-and-source-mutations', () => {
    const proxy = {...copy(ss), name: '', resolved: false, unknown: {nil: null, zero: 0, flag: false}};
    const result = utils.produce([proxy], 'JSON', 'internal');
    assert(result[0] === proxy, 'JSON internal retains proxy identity');
    equal(proxy.name, 'ss ss.example.com:8388', 'missing name filled on caller object');
    assert(proxy._resolved === false, 'resolved boolean retained');
    const object = JSON.parse(utils.produce(result, 'json'));
    equal(object, result, 'external exact JSON');
    const websocket = {};
    for (const target of ['Clash', ...aliases[5], 'Stash', ...aliases[10]]) {
      const shared = {path: '/socket?token=keep&ed=2048', headers: {Host: 'cdn.example'}, extra: {keep: true}};
      const source = {type: 'vmess', name: 'shared', server: 'ws.example', port: 443, uuid, network: 'ws', 'ws-opts': shared};
      const before = copy(shared);
      utils.produce([{...source}], 'JSON', 'internal');
      equal(shared, before, 'JSON does not apply a platform transport mutation');
      const caller = {...source}, headers = shared.headers, extra = shared.extra;
      const produced = utils.produce([caller], target, 'internal');
      assert(caller['ws-opts'] === shared && shared.headers === headers && shared.extra === extra, 'shared transport identity ' + target);
      equal(shared.path, '/socket?token=keep', 'shared early data path ' + target);
      equal(shared['max-early-data'], 2048, 'shared early data amount ' + target);
      equal(shared['early-data-header-name'], 'Sec-WebSocket-Protocol', 'shared early data header ' + target);
      equal(produced.length, 1, 'shared producer output ' + target);
      websocket[target] = copy(shared);
    }
    const filtered = {type: 'h2-connect', name: 'filtered', server: 'ws.example', port: 443, network: 'ws', 'ws-opts': {path: '/ws?ed=1024'}};
    equal(utils.produce([{...filtered}], 'ClashMeta', 'internal'), [], 'target-specific filter precedes transport effects');
    equal(filtered['ws-opts'], {path: '/ws?ed=1024'}, 'filtered transport is untouched');
    const upgrade = {type: 'vmess', name: 'upgrade', server: 'ws.example', port: 443, uuid, network: 'ws', 'ws-opts': {
      path: '/ws?ed=4096', 'v2ray-http-upgrade': true, 'early-data-header-name': 'old', 'max-early-data': 1}};
    utils.produce([{...upgrade}], 'ClashMeta', 'internal');
    equal(upgrade['ws-opts'], {path: '/ws', 'v2ray-http-upgrade': true, 'v2ray-http-upgrade-fast-open': true, '_v2ray-http-upgrade-ed': '4096'}, 'shared upgrade migration and deletions');
    const retainedUpgrade = copy(upgrade['ws-opts']);
    utils.produce([{...upgrade}], 'ClashMeta', 'external');
    assert(!Object.hasOwn(upgrade['ws-opts'], '_v2ray-http-upgrade-ed'), 'external producer deletes shared private metadata');
    const empty = {type: 'vmess', name: 'empty', server: 'ws.example', port: 443, uuid, network: 'ws', 'ws-opts': {path: '', 'max-early-data': 0}};
    utils.produce([{...empty}], 'ClashMeta', 'internal');
    equal(empty['ws-opts'], {path: '/', 'max-early-data': 0}, 'falsy path defaults while explicit early data remains');
    return {result, external: utils.produce(result, 'JSON'), websocket, upgrade: retainedUpgrade};
  });
  await check('PC98-conversion-platform-filtering-and-overrides', () => {
    const supported = {...copy(ss), supported: {ClashMeta: false}};
    equal(utils.produce([supported], 'ClashMeta', 'internal'), [], 'explicit target support filter');
    equal(utils.produce([copy(supported)], 'ClashMeta', 'internal', {'include-unsupported-proxy': true}).length, 1, 'support override');
    const root = {type: 'http', name: 'headers', server: 'h.example', port: 8080, headers: {Authorization: 'fixture'}, udp: false};
    equal(utils.produce([copy(root)], 'Loon', 'internal'), [], 'unsupported root headers filtered');
    const reality = {type: 'vless', name: 'bad', server: 'v.example', port: 443, uuid, tls: true, 'reality-opts': {'short-id': '08'}};
    equal(utils.produce([reality], 'ClashMeta', 'internal'), [], 'empty Reality key filtered');
    const split = {...copy(reality), network: 'xhttp', 'reality-opts': {'public-key': 'key'}, 'xhttp-opts': {mode: 'stream-one', 'download-settings': {}}};
    equal(utils.produce([split], 'ClashMeta', 'internal'), [], 'invalid xhttp split filtered');
    const shadow = {...copy(ss), tls: true};
    equal(utils.produce([copy(shadow)], 'Surge', 'internal'), [], 'Shadowsocks-over-TLS target filter');
    return {supported: utils.produce([copy(supported)], 'ClashMeta', 'internal', {'include-unsupported-proxy': true}), headers: utils.produce([root], 'JSON', 'internal')};
  });
  await check('PC99-conversion-empty-formats-and-type-boundaries', () => {
    const empty = {};
    for (const [platform] of aliases) {
      const internal = utils.produce([], platform, 'internal');
      equal(internal, platform === 'v2' ? '' : [], 'empty internal ' + platform);
      const external = utils.produce([], platform);
      assert(typeof external === 'string', 'empty external ' + platform);
      empty[platform] = external;
    }
    rejects(() => utils.produce([], 'unsupported'), Error, 'Target platform: unsupported is not supported!');
    rejects(() => utils.produce(null, 'JSON'), TypeError);
    rejects(() => utils.produce([null], 'JSON'), TypeError);
    return empty;
  });
  await check('PC100-conversion-options-and-single-proxy-failure', () => {
    const proxy = {...copy(ss), 'disable-sni': true};
    const surge = utils.produce([copy(proxy)], 'Surge', 'internal');
    const mihomo = utils.produce([copy(proxy)], 'Mihomo', 'internal');
    const singbox = utils.produce([copy(ss)], 'sing-box', 'internal', {'include-unsupported-proxy': true});
    equal(utils.produce([{type: 'unsupported', name: 'bad', server: 'x', port: 1}, copy(ss)], 'URI'), utils.produce([copy(ss)], 'URI'), 'SINGLE producer skips failed proxy and recovers');
    return {surge, mihomo, singbox};
  });
  await check('PC101-conversion-port-hopping-and-surgetitles', () => {
    const hy = {type: 'hysteria2', name: 'hopping', server: 'hy.example', password: 'secret', ports: '443-443/444', port: 443, 'skip-cert-verify': true};
    const mihomo = utils.produce([copy(hy)], 'Mihomo', 'internal');
    const surge = utils.produce([copy(hy)], 'Surge', 'internal');
    const wg = {type: 'wireguard', name: 'wg', server: 'wg.example', port: 51820, ip: '10.0.0.2/32', 'private-key': 'private', 'public-key': 'public', _subName: '订阅🙂', _desc: '', _category: 'fixture'};
    const title = utils.produce([copy(wg)], 'Surge');
    assert(title.startsWith('#!name=订阅🙂\n#!desc=\n#!category=fixture\n'), 'WireGuard subscription metadata');
    return {mihomo, surge, title};
  });
  await check('PC102-conversion-share-roundtrip-and-exact-uri', () => {
    const external = utils.produce([copy(ss)], 'URI');
    equal(external, 'ss://YWVzLTEyOC1nY206c2VjcmV0@ss.example.com:8388?udp=1#%E8%8A%82%E7%82%B9%F0%9F%99%82', 'exact SIP002 external bytes');
    equal(utils.parse(external), [ss], 'SS roundtrip');
    const vless = {type: 'vless', name: 'VLESS🙂', server: 'v.example', port: 443, uuid, udp: true, tls: true, network: 'ws', 'ws-opts': {path: '/ws', headers: {Host: 'cdn.example'}}, sni: 'cdn.example'};
    const vlessUri = utils.produce([copy(vless)], 'URI');
    const parsed = one(vlessUri, {type: 'vless', name: vless.name, uuid, tls: true, network: 'ws'});
    return {external, vlessUri, parsed};
  });
  await check('PC103-conversion-in-pending-script-and-reentry', async () => {
    const result = await probeReenter(JSON.stringify({raw: uri}), async payload => {
      const raw = JSON.parse(payload).raw;
      const output = await operator(`async function operator(proxies) {
        await $substore.wait(1);
        const parsed = ProxyUtils.parse($arguments.raw);
        const external = ProxyUtils.produce(parsed, 'JSON');
        $substore.write({external, flag: false, zero: 0}, 'conversionHelper');
        return JSON.parse(external);
      }`, [], {raw});
      return JSON.stringify(output);
    });
    const expected = [{...ss}];
    // JSON producer adds an own undefined _resolved before serialization.
    equal(JSON.parse(result), expected, 'actual Script Operator invokes synchronous conversion while native reentry is pending');
    equal($substore.read('conversionHelper').flag, false, 'persistent false');
    return JSON.parse(result);
  });
  await check('PC104-conversion-invalid-inputs-and-recovery', () => {
    for (const input of [undefined, null, false, 0, [], {}]) rejects(() => utils.parse(input), TypeError);
    equal(utils.parse(''), [], 'empty input');
    equal(utils.parse('invalid://not-a-proxy\nss://bad\n' + uri), [ss], 'malformed input does not poison parser state');
    equal(utils.parse(JSON.stringify({...ss, type: 'hysteria2', obfs: 'salamander', 'obfs-password': ''})), [], 'missing obfs password filtered');
    return {rejections: 6, recovered: one(uri)};
  });
};
