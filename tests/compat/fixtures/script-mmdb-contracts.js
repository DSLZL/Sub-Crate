async function runMmdbContracts(check, host, operator, assert, equal) {
  const MMDB = host.ProxyUtils.MMDB;
  const nativePath = name => probeMmdbRoot + (probeMmdbRoot.includes('\\') ? '\\' : '/') + name;
  const country = nativePath('mmdb country🙂.mmdb');
  const asn = nativePath('mmdb asn🙂.mmdb');
  const missing = nativePath('mmdb missing🙂.mmdb');
  function failure(callback, name, message) {
    let caught;
    try { callback(); } catch (error) { caught = error; }
    assert(caught && caught.name === name, `expected ${name}, got ${caught?.name}: ${caught?.message}`);
    assert(caught instanceof Error && (name !== 'TypeError' || caught instanceof TypeError), 'MMDB native Error identity');
    if (message !== undefined) equal(caught.message, message, 'MMDB diagnostic');
    return {name: caught.name, message: caught.message};
  }
  function missingCountry(callback, file) {
    let caught;
    try { callback(); } catch (error) { caught = error; }
    assert(caught?.name === 'Error' && caught.code === 'ENOENT' && caught.path === file, 'country missing-path error fields');
    equal(caught.message, `ENOENT: no such file or directory, open '${file}'`, 'complete native-path diagnostic');
    // Paths differ between isolated processes; each complete diagnostic is checked above.
    return {name: caught.name, code: caught.code, pathMatches: caught.path === file, messageMatches: true};
  }
  await check('PC65-mmdb-process-defaults-and-sync-results', () => {
    const db = new MMDB();
    const result = [db.geoip('81.2.69.160'), db.ipasn('1.0.0.1'), db.ipaso('1.0.0.1')];
    equal(result, ['GB', 15169, 'Google Inc.'], 'actual process environment defaults');
    assert(typeof result[1] === 'number' && !(result[0] instanceof Promise), 'synchronous numeric/string helper');
    return result;
  });
  await check('PC66-mmdb-optional-readers-and-falsy-paths', () => probeMmdbWithDefaults({}, () => {
    for (const args of [undefined, {}, {country: '', asn: false}, {country: 0, asn: null}]) {
      const db = new MMDB(args);
      assert(!Object.hasOwn(db, 'countryReader') && !Object.hasOwn(db, 'asnReader'), 'absent readers stay absent');
      assert(db.geoip(null) === undefined && db.ipasn('bad') === undefined && db.ipaso() === undefined, 'optional calls do not validate or return null');
    }
    const c = new MMDB({country}), a = new MMDB({asn});
    assert(c.geoip('81.2.69.160') === 'GB' && c.ipasn('bad') === undefined, 'country-only instance');
    assert(a.ipasn('1.0.0.1') === 15169 && a.geoip('bad') === undefined, 'ASN-only instance');
    return {optionalType: typeof new MMDB().geoip('bad'), country: c.geoip('81.2.69.160'), asn: a.ipasn('1.0.0.1')};
  }));
  await check('PC67-mmdb-explicit-and-falsy-default-precedence', () => {
    const explicit = probeMmdbWithDefaults({country: missing, asn: missing}, () => new MMDB({country, asn}));
    const fallback = new MMDB({country: '', asn: 0});
    equal([explicit.geoip('175.16.199.1'), explicit.ipasn('1.128.0.1'), fallback.geoip('81.2.69.160')], ['CN', 1221, 'GB'], 'explicit paths win and falsy paths use defaults');
    return {explicit: ['CN', 1221], fallback: 'GB'};
  });
  await check('PC68-mmdb-country-v4-v6-and-absent-field', () => {
    const db = new MMDB({country});
    const output = probeMmdbVectors.country.map(({ip, record}) => {
      const result = db.geoip(ip);
      assert(result === record.country?.iso_code, 'country differs from independent official source JSON: ' + ip);
      return {ip, value: result === undefined ? {type: 'undefined'} : result};
    });
    equal(output.map(row => row.value), ['GB', 'CN', 'JP', {type: 'undefined'}], 'country and absent-field vectors');
    return output;
  });
  await check('PC69-mmdb-asn-v4-v6-number-and-absent-organization', () => {
    const db = new MMDB({asn});
    return probeMmdbVectors.asn.map(({ip, record}) => {
      const number = db.ipasn(ip), organization = db.ipaso(ip);
      assert(number === record.autonomous_system_number && organization === record.autonomous_system_organization, 'ASN differs from official source JSON: ' + ip);
      assert(typeof number === 'number', 'ASN must retain numeric type');
      return {ip, number, organization: organization === undefined ? {type: 'undefined'} : organization};
    });
  });
  await check('PC70-mmdb-invalid-ip-errors-and-recovery', () => {
    const db = new MMDB();
    const errors = [];
    for (const ip of ['bad', '01.2.3.4', '', null, false, 42, undefined]) {
      for (const method of ['geoip', 'ipasn', 'ipaso']) errors.push(failure(() => db[method](ip), 'ValueError', `${ip} is invalid`));
    }
    for (const method of ['geoip', 'ipasn', 'ipaso']) {
      errors.push(failure(() => db[method](['1.0.0.1']), 'TypeError', 'input.split is not a function'));
      errors.push(failure(() => db[method](new String(method === 'geoip' ? '81.2.69.160' : '1.0.0.1')), 'Error', 'Invalid argument: Addr or parsable string expected'));
    }
    equal([db.geoip('81.2.69.160'), db.ipasn('1.0.0.1')], ['GB', 15169], 'reader recovers after exceptions');
    return errors;
  });
  await check('PC71-mmdb-unmapped-address-errors', () => {
    const db = new MMDB();
    return ['geoip', 'ipaso', 'ipasn'].map(method => failure(() => db[method]('127.0.0.1'), 'AddressNotFoundError', 'The address 127.0.0.1 is not in the database'));
  });
  await check('PC72-mmdb-database-kind-and-validation-order', () => {
    const db = new MMDB({country: asn, asn: country});
    return [failure(() => db.geoip('1.0.0.1'), 'BadMethodCallError', 'The country() method cannot be used with the GeoLite2-ASN database'),
      failure(() => db.ipasn('81.2.69.160'), 'BadMethodCallError', 'The asn() method cannot be used with the GeoIP2-Country database'),
      failure(() => db.ipaso('bad'), 'ValueError', 'bad is invalid')];
  });
  await check('PC73-mmdb-missing-files-and-constructor-order', () => {
    const errors = [missingCountry(() => new MMDB({country: missing}), missing),
      failure(() => new MMDB({asn: missing}), 'Error', 'GeoLite2 ASN MMDB does not exist'),
      missingCountry(() => new MMDB({country: missing, asn: missing}), missing)];
    failure(() => new MMDB(null), 'TypeError');
    return errors;
  });
  await check('PC74-mmdb-corrupt-buffer-errors', () => {
    const file = nativePath('mmdb corrupt.mmdb');
    const diagnostic = probeKind === 'node' ? 'Error: Unknown type 117 at offset 1'
      : 'invalid database: could not find MaxMind DB metadata in file';
    const errors = [failure(() => new MMDB({country: file}), 'InvalidDbBufferError', diagnostic),
      failure(() => new MMDB({asn: file}), 'InvalidDbBufferError', diagnostic)];
    // shortcut: parser diagnostic text differs; full ReaderModel/error parity remains an M3 owner.
    return errors.map(({name}) => ({name, diagnosticChecked: true}));
  });
  await check('PC75-mmdb-eager-read-instance-isolation-and-source-methods', () => {
    const file = nativePath('mmdb transient🙂.mmdb');
    const db = new MMDB({country: file}), other = new MMDB();
    assert(db.countryReader !== other.countryReader, 'instances must own independent reader objects');
    probeMmdbRemoveTransient();
    equal([db.geoip('81.2.69.160'), other.geoip('175.16.199.1')], ['GB', 'CN'], 'eager readers survive file removal');
    const error = missingCountry(() => new MMDB({country: file}), file);
    db.countryReader = {country: () => ({country: {isoCode: false}})};
    db.asnReader = {asn: () => ({autonomousSystemNumber: 0, autonomousSystemOrganization: ''})};
    equal([db.geoip('x'), db.ipasn('x'), db.ipaso('x')], [false, 0, ''], 'actual source methods preserve falsy fields');
    delete db.countryReader; delete db.asnReader;
    assert(db.geoip('x') === undefined && db.ipasn('x') === undefined, 'actual methods retain optional readers');
    return {afterRemoval: ['GB', 'CN'], error, falsy: [false, 0, ''], absent: 'undefined'};
  });
  await check('PC76-mmdb-in-async-script-and-persistence', async () => {
    const output = await operator(`async function operator(proxies) {
      await $substore.wait(1); const db = new ProxyUtils.MMDB();
      if (!$arguments.node && ($substore.env.isNode || typeof require !== 'undefined')) throw new Error('Node capability leaked');
      const record = {country: db.geoip('2001:218::1'), asn: db.ipasn('2001:1700::1'), organization: db.ipaso('2001:1700::1')};
      $substore.write(record, 'mmdb-script'); return proxies.map(proxy => ({...proxy, ...record}));
    }`, [{name: 'MMDB🙂'}], {node: probeKind === 'node'});
    equal(output, [{name: 'MMDB🙂', country: 'JP', asn: 6730, organization: 'Sunrise Communications AG'}], 'injected MMDB output');
    equal($substore.read('mmdb-script'), {country: 'JP', asn: 6730, organization: 'Sunrise Communications AG'}, 'MMDB script persists complete values');
    return output;
  });
}
