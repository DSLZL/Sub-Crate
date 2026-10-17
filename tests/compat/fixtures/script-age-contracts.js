(async () => {
  const passed = [], observations = {}, verified = [];
  const api = ageProbeUtils;
  const payloads = [
    {id: 'empty', text: ''},
    {id: 'unicode', text: '订阅🙂\r\n\0尾\n'},
    {id: 'view', text: '视图🙂\0\r\n'},
    {id: 'chunk-boundary', text: 'a'.repeat(65535) + '🙂尾'},
  ];
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const equal = (actual, expected, message) => assert(JSON.stringify(actual) === JSON.stringify(expected), message);
  const reject = async (callback, predicate, message) => {
    try { await callback(); } catch (error) { assert(predicate(error), message + ': ' + error.message); return error; }
    throw new Error(message + ': did not reject');
  };
  async function check(id, callback) {
    probeAgeProgress(id, 'start');
    observations[id] = await callback(); passed.push(id);
    probeAgeProgress(id, 'end');
  }
  const bytes = text => new TextEncoder().encode(text);
  const plaintext = item => {
    if (item.id !== 'view') return item.text;
    const data = bytes('x' + item.text + 'y');
    return data.subarray(1, data.length - 1);
  };
  async function encryptItems(publicKey) {
    const items = [];
    for (const item of payloads) items.push({id: item.id, armor: await api.encryptArmor(plaintext(item), publicKey)});
    return items;
  }
  async function openItems(pair, items, direction) {
    equal(items.map(item => item.id), payloads.map(item => item.id), 'exact exchange payloads');
    assert(await api.derivePublicKey(pair.secret) === pair.public, 'exchange identity/recipient match');
    for (let index = 0; index < payloads.length; index++) {
      const actual = await api.decryptArmorIfPresent(items[index].armor, pair.secret);
      equal(actual, payloads[index].text, direction + ' plaintext bytes');
      verified.push({direction, type: pair.type, id: items[index].id, utf8Length: bytes(actual).length,
        md5: processorHost.ProxyUtils.hex_md5(actual)});
    }
  }
  const types = ['x25519', 'mlkem768-x25519'];
  async function openPeer(peer, own) {
    if (peer.pairs) {
      equal(peer.pairs.map(pair => pair.type), types, 'exact peer key types');
      for (const pair of peer.pairs) await openItems(pair, pair.items, peer.sender + '-key-to-' + probeKind);
    }
    if (peer.replies) {
      equal(peer.replies.map(reply => reply.type), types, 'exact reply key types');
      for (let index = 0; index < types.length; index++) {
        const pair = own.pairs[index], reply = peer.replies[index];
        assert(reply.public === pair.public && reply.type === pair.type, 'reply addressed to generated key');
        await openItems(pair, reply.items, peer.sender + '-to-' + probeKind + '-key');
      }
    }
  }
  const exchange = {sender: probeKind};
  if (ageInput.mode === 'full') {
    await check('AG01-encoding-utf8-views-fatal-stream-and-bom', async () => {
      const encoded = bytes('订阅🙂');
      equal(Array.from(encoded), [232,174,162,233,152,133,240,159,153,130], 'UTF-8 vector');
      equal(new TextDecoder().decode(new DataView(encoded.buffer, 3, 3)), '阅', 'view offset');
      const decoder = new TextDecoder('utf-8', {fatal: true});
      equal(decoder.decode(encoded.subarray(0, 8), {stream: true}), '订阅', 'split code point');
      equal(decoder.decode(encoded.subarray(8)), '🙂', 'stream flush');
      await reject(() => decoder.decode(new Uint8Array([0xff])), error => error instanceof TypeError, 'fatal input');
      equal(decoder.decode(encoded), '订阅🙂', 'decoder recovers');
      equal(new TextDecoder().decode(new Uint8Array([239,187,191,65])), 'A', 'BOM removal');
      equal(new TextDecoder('utf-8', {ignoreBOM: true}).decode(new Uint8Array([239,187,191,65])), '\uFEFFA', 'BOM preserved');
      equal(new TextDecoder().decode(new Uint8Array([0xff])), '\uFFFD', 'replacement');
      return {utf8: Array.from(encoded), fatal: true, stream: true, bom: true};
    });
    await check('AG02-stream-backpressure-cancel-and-errors', async () => {
      const events = [];
      const transform = new TransformStream({transform(chunk, controller) { events.push(chunk); controller.enqueue(chunk * 2); }});
      const writer = transform.writable.getWriter(), reader = transform.readable.getReader();
      let settled = false;
      const pending = writer.write(3).then(() => { settled = true; });
      await Promise.resolve(); assert(!settled, 'write waits for reader');
      equal(await reader.read(), {value: 6, done: false}, 'transform output'); await pending;
      await writer.close(); equal(await reader.read(), {done: true}, 'closed stream');
      let reason;
      await new ReadableStream({cancel(value) { reason = value; }}).cancel('owned cancel');
      equal(reason, 'owned cancel', 'cancel callback');
      const failure = new Error('controlled stream failure');
      const broken = new ReadableStream({start(controller) { controller.error(failure); }});
      await reject(() => broken.getReader().read(), error => error === failure, 'stream failure identity');
      return {events, canceled: reason, failure: failure.message};
    });
    await check('AG03-response-bodies-once-offsets-and-errors', async () => {
      const response = new Response(new ReadableStream({start(controller) {
        controller.enqueue(bytes('订')); controller.enqueue(bytes('阅🙂')); controller.close();
      }}));
      equal(await response.text(), '订阅🙂', 'stream body'); assert(response.bodyUsed, 'body consumed');
      await reject(() => response.arrayBuffer(), error => error instanceof TypeError, 'second consumption');
      const view = bytes('x视图y').subarray(1, 7);
      equal(Array.from(new Uint8Array(await new Response(view).arrayBuffer())), Array.from(view), 'body view offset');
      equal(await new Response('{"zero":0,"flag":false}').json(), {zero: 0, flag: false}, 'JSON body');
      const failure = new Error('controlled body failure');
      await reject(() => new Response(new ReadableStream({start(controller) { controller.error(failure); }})).text(),
        error => error === failure, 'body error propagated');
      return {text: '订阅🙂', oneShot: true, view: Array.from(view), error: failure.message};
    });
    await check('AG04-system-random-quota-types-and-views', async () => {
      for (const Type of [Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array, Uint32Array, BigInt64Array, BigUint64Array]) {
        const value = new Type(4); assert(crypto.getRandomValues(value) === value, 'same random array');
      }
      const parent = new Uint8Array(34).fill(91), view = parent.subarray(1, 33);
      crypto.getRandomValues(view);
      assert(parent[0] === 91 && parent[33] === 91, 'random only touches view');
      const second = crypto.getRandomValues(new Uint8Array(32));
      assert(view.some((value, index) => value !== second[index]), 'independent random bytes');
      assert(crypto.getRandomValues(new Uint8Array(0)).length === 0, 'zero length');
      assert(crypto.getRandomValues(new Uint8Array(65536)).length === 65536, 'inclusive byte quota');
      await reject(() => crypto.getRandomValues(new Uint8Array(65537)), error => error.name === 'QuotaExceededError', 'byte quota');
      for (const value of [[], new Float32Array(1), new DataView(new ArrayBuffer(1)), new ArrayBuffer(1)]) {
        await reject(() => crypto.getRandomValues(value), error => error.name === 'TypeMismatchError', 'integer views only');
      }
      const spoofed = new Float32Array(1);
      Object.defineProperty(spoofed, Symbol.toStringTag, {value: 'Uint8Array'});
      await reject(() => crypto.getRandomValues(spoofed), error => error.name === 'TypeMismatchError', 'spoofed view rejected');
      if (probeKind === 'rust') {
        // Node 24 trusts an overridden byteLength; the native boundary must keep its quota.
        const oversized = new Uint8Array(65537);
        Object.defineProperty(oversized, 'byteLength', {value: 1});
        await reject(() => crypto.getRandomValues(oversized), error => error.name === 'QuotaExceededError', 'quota uses internal length');
        await reject(() => __hostRandomValues(65537), error => error instanceof Error, 'native quota cannot be bypassed');
      }
      return {types: 9, view: true, independent: true, quota: 65536};
    });
    exchange.pairs = [];
    await check('AG05-generate-x25519-and-hybrid-derive-and-normalize', async () => {
      for (const type of types) {
        const pair = await api.generateKeyPair(type), secret = pair['age-secret-key'], publicKey = pair['age-public-key'];
        assert(secret.startsWith(type === types[0] ? 'AGE-SECRET-KEY-1' : 'AGE-SECRET-KEY-PQ-1'), 'identity prefix');
        assert(publicKey.startsWith(type === types[0] ? 'age1' : 'age1pq1'), 'recipient prefix');
        assert(await api.derivePublicKey(' \n' + secret + '\r\n') === publicKey, 'normalized derive');
        const config = {'age-public-key': ' ' + publicKey + ' ', 'age-secret-key': '\n' + secret + '\n', unknown: false};
        assert(api.normalizeAgePublicKeyConfig(config) === config, 'public mutates config');
        assert(await api.normalizeAgeSecretKeyConfig(config) === config, 'secret mutates config');
        equal(config, {'age-public-key': publicKey, 'age-secret-key': secret, unknown: false}, 'config retained');
        exchange.pairs.push({type, secret, public: publicKey});
      }
      return types;
    });
    await check('AG06-armor-roundtrip-unicode-view-and-chunk-boundary', async () => {
      for (const pair of exchange.pairs) {
        pair.items = await encryptItems(pair.public);
        for (let index = 0; index < payloads.length; index++) {
          assert(api.isAgeArmor(pair.items[index].armor), 'armored output');
          equal(await api.decryptArmorIfPresent(pair.items[index].armor, pair.secret), payloads[index].text, 'roundtrip');
        }
        const one = await api.encryptArmor(null, pair.public);
        equal(await api.decryptArmorIfPresent(one, pair.secret), '', 'null plaintext');
        assert(one !== pair.items[0].armor, 'same plaintext uses independent encryption randomness');
      }
      return payloads.map(item => ({id: item.id, utf8Length: bytes(item.text).length}));
    });
    await check('AG07-non-armor-preserves-types-and-identity', async () => {
      for (const input of ['', 'ordinary🙂', bytes('ordinary🙂'), new Uint8Array([255, 0])]) {
        assert(await api.decryptArmorIfPresent(input, 'invalid') === input, 'non-armor passes through');
      }
      equal(await api.decryptArmorIfPresent(null, 'invalid'), '', 'null pass-through');
      return {sameReference: true, null: ''};
    });
    await check('AG08-validation-wrong-keys-tampering-and-recovery', async () => {
      for (const key of ['', 'age1invalid', 'age1tag1invalid', 'age1tagpq1invalid']) {
        await reject(() => api.encryptArmor('x', key), error => error.code === 'INVALID_AGE_PUBLIC_KEY' && error.type === 'RequestInvalidError', 'public validation');
      }
      for (const key of ['', 'AGE-SECRET-KEY-1INVALID', 'AGE-SECRET-KEY-PQ-1INVALID']) {
        await reject(() => api.derivePublicKey(key), error => error.code === 'INVALID_AGE_SECRET_KEY', 'secret validation');
      }
      await reject(() => api.generateKeyPair('tagpq'), error => error.message === '不支持的 age key 类型: tagpq. 支持: x25519, mlkem768-x25519', 'unsupported type');
      for (let index = 0; index < exchange.pairs.length; index++) {
        const pair = exchange.pairs[index], armor = pair.items[1].armor;
        await reject(() => api.decryptArmorIfPresent(armor, exchange.pairs[1 - index].secret), error => error.message.startsWith('age 解密失败:'), 'wrong identity');
        const lines = armor.trimEnd().split('\n');
        const last = lines.length - 2;
        lines[last] = (lines[last][0] === 'A' ? 'B' : 'A') + lines[last].slice(1);
        await reject(() => api.decryptArmorIfPresent(lines.join('\n') + '\n', pair.secret), error => error.message.startsWith('age 解密失败:'), 'authenticated ciphertext');
        await reject(() => api.decryptArmorIfPresent('-----BEGIN AGE ENCRYPTED FILE-----\nbad\n', pair.secret), error => error.message.startsWith('age 解密失败:'), 'broken armor');
        equal(await api.decryptArmorIfPresent(bytes(armor), pair.secret), payloads[1].text, 'binary armor recovers');
      }
      return {publicError: 'INVALID_AGE_PUBLIC_KEY', secretError: 'INVALID_AGE_SECRET_KEY', tampering: true, recovered: true};
    });
    await check('AG09-secret-redaction-and-empty-config', async () => {
      for (const pair of exchange.pairs) {
        const prefix = pair.type === types[0] ? 'AGE-SECRET-KEY-1' : 'AGE-SECRET-KEY-PQ-1';
        equal(api.maskAgeSecret('secret=' + pair.secret), 'secret=' + prefix + '...' + pair.secret.slice(-6), 'key mask');
        equal(api.maskAgeSecretInUrl('https://fixture.invalid/?age-secret-key=' + pair.secret + '&x=0'), 'https://fixture.invalid/?age-secret-key=***&x=0', 'URL secret mask');
      }
      const config = {'age-public-key': ' ', 'age-secret-key': null, unknown: false};
      api.normalizeAgePublicKeyConfig(config); await api.normalizeAgeSecretKeyConfig(config);
      equal(config, {unknown: false}, 'empty keys removed');
      return {masked: true, emptyConfig: config};
    });
    await check('AG10-public-age-in-actual-async-script-operator', async () => {
      const pair = exchange.pairs[0];
      const factory = processorHost.factories['Script Operator'];
      const processor = factory(`async function operator(proxies) {
        const {encrypt, decrypt, derivePublicKey} = ProxyUtils.age;
        if (!$arguments.node && (typeof require !== 'undefined' || typeof process !== 'undefined' || $substore.env.isNode)) throw new Error('Node leaked');
        const key = await derivePublicKey($arguments.secret);
        const armor = await encrypt('脚本🙂\\r\\n', key);
        proxies[0].text = await decrypt(armor, $arguments.secret);
        return proxies;
      }`, 'ClashMeta', {secret: pair.secret, node: probeKind === 'node'}, {}, {}, {});
      const result = await processorHost.ApplyProcessor(processor, [{zero: 0, flag: false}]);
      equal(result, [{zero: 0, flag: false, text: '脚本🙂\r\n'}], 'public helper injection');
      return result;
    });
    await check('AG11-missing-runtime-diagnostics-before-crypto', async () => {
      const required = ['TextEncoder', 'TextDecoder', 'ReadableStream', 'TransformStream', 'Response', 'Uint8Array'];
      const root = Object.fromEntries(required.map(name => [name, globalThis[name]]));
      root.crypto = crypto;
      delete root.Response;
      await reject(() => api.assertAgeRuntimeAvailable(root), error =>
        error.message === '当前运行环境缺少 age 加解密所需 Web API: Response', 'one missing API');
      const message = '当前运行环境缺少 age 加解密所需 Web API: ' + [...required, 'crypto.getRandomValues'].join(', ');
      await reject(() => api.assertAgeRuntimeAvailable({}), error => error.message === message, 'complete missing API list');
      return {missing: required, random: 'crypto.getRandomValues'};
    });
  }
  if (ageInput.peer) await openPeer(ageInput.peer, ageInput.own);
  if (ageInput.peer?.pairs) {
    exchange.replies = [];
    for (const pair of ageInput.peer.pairs) exchange.replies.push({type: pair.type, public: pair.public, items: await encryptItems(pair.public)});
  }
  return {passed, observations, verified, exchange};
})()
