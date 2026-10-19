async function operator(proxies) {
  const api=ProxyUtils.age, pairs=$arguments.pairs;
  const passed=[], results={};
  const assert=(value,message)=>{if(!value) throw new Error(message);};
  const equal=(a,b,message)=>assert(JSON.stringify(a)===JSON.stringify(b),message);
  const check=async(id,callback)=>{results[id]=await callback();passed.push(id);};
  const reject=async(callback,predicate)=>{
    try {await callback();} catch(error) {assert(predicate(error),'age error: '+error?.message);return;}
    throw new Error('age operation did not reject');
  };
  const texts={empty:'',unicode:'订阅🙂\r\n\0尾\n',view:'视图🙂\0\r\n'};
  await check('AG-native-public-contract-and-normalized-keys',async()=>{
    equal(Object.keys(api).sort(),['decrypt','derivePublicKey','encrypt'],'public API');
    for(const pair of pairs) equal(await api.derivePublicKey(' \n'+pair.secret+'\r\n'),pair.public,'normalized key');
    return pairs.map(pair=>pair.type);
  });
  await check('AG-native-independent-node-ciphertexts',async()=>{
    for(const pair of pairs) for(const item of pair.items) {
      equal(await api.decrypt(item.armor,pair.secret),texts[item.id],'Node ciphertext');
      const bytes=Buffer.from('x'+item.armor+'y');
      equal(await api.decrypt(bytes.subarray(1,bytes.length-1),pair.secret),texts[item.id],'armored view');
    }
    return {types:2,payloads:6,offsetViews:6};
  });
  await check('AG-native-live-entropy-and-chunk-boundary',async()=>{
    const large='a'.repeat(65535)+'🙂尾';
    for(const pair of pairs) {
      const one=await api.encrypt(texts.unicode,pair.public),two=await api.encrypt(texts.unicode,pair.public);
      assert(one!==two,'encryption uses independent system entropy');
      equal(await api.decrypt(one,pair.secret),texts.unicode,'fresh roundtrip');
      equal(await api.decrypt(await api.encrypt(null,pair.public),pair.secret),'','null plaintext');
      const bytes=Buffer.from('x'+texts.view+'y');
      equal(await api.decrypt(await api.encrypt(bytes.subarray(1,bytes.length-1),pair.public),pair.secret),texts.view,'plaintext view');
      equal(await api.decrypt(await api.encrypt(large,pair.public),pair.secret),large,'chunk boundary');
    }
    return {independentEntropy:true,chunkBoundary:65535,types:2};
  });
  await check('AG-native-pass-through-and-validation',async()=>{
    for(const value of ['',false,0,'plain🙂',Buffer.from([255,0])]) {
      // Non-string primitives follow TextDecoder's input validation.
      if(typeof value==='boolean'||typeof value==='number') {
        await reject(()=>api.decrypt(value,'invalid'),error=>error instanceof TypeError);
      } else assert(await api.decrypt(value,'invalid')===value,'plain value retains identity');
    }
    equal(await api.decrypt(null,'invalid'),'','null pass-through');
    for(const value of ['', 'age1invalid','age1tag1invalid','age1tagpq1invalid',null,0])
      await reject(()=>api.encrypt('x',value),error=>error.code==='INVALID_AGE_PUBLIC_KEY'&&error.type==='RequestInvalidError');
    for(const value of ['', 'AGE-SECRET-KEY-1INVALID','AGE-SECRET-KEY-PQ-1INVALID',null,0])
      await reject(()=>api.derivePublicKey(value),error=>error.code==='INVALID_AGE_SECRET_KEY'&&error.type==='RequestInvalidError');
    for(let i=0;i<pairs.length;i++) {
      await reject(()=>api.decrypt(pairs[i].items[1].armor,pairs[1-i].secret),error=>error.message.startsWith('age 解密失败:'));
      await reject(()=>api.decrypt('-----BEGIN AGE ENCRYPTED FILE-----\nbad\n',pairs[i].secret),error=>error.message.startsWith('age 解密失败:'));
      equal(await api.decrypt(pairs[i].items[1].armor,pairs[i].secret),texts.unicode,'recovery');
    }
    return {identity:true,validation:true,recovery:true};
  });
  await check('AG-native-required-runtime-diagnostics',async()=>{
    for(const name of ['TextEncoder','TextDecoder','ReadableStream','TransformStream','Response','Uint8Array','crypto']) {
      const descriptor=Object.getOwnPropertyDescriptor(globalThis,name);
      try {
        Object.defineProperty(globalThis,name,{value:undefined,writable:true,configurable:true});
        await reject(()=>api.encrypt('x',pairs[0].public),error=>error.message==='当前运行环境缺少 age 加解密所需 Web API: '+(name==='crypto'?'crypto.getRandomValues':name));
        equal(await api.decrypt('plain','invalid'),'plain','plaintext does not require crypto runtime');
      } finally { Object.defineProperty(globalThis,name,descriptor); }
    }
    return {missingBeforeCrypto:true,plainPassThrough:true};
  });
  const OriginalDate=Date;
  globalThis.Date=class extends OriginalDate {
    constructor(...args){super(...(args.length?args:[3100000000000]));}
    static now(){return 3100000000000;}
  };
  try {
    await check('AG-native-download-auto-decrypt-and-original-cache',async()=>{
      for(const pair of pairs) {
        const url='$UPSTREAM/host/download/age/'+pair.type;
        const value=await ProxyUtils.download(url,undefined,undefined,undefined,undefined,undefined,undefined,false,{ageSecretKey:pair.secret,returnRaw:true});
        equal(value,{result:texts.unicode,raw:texts.unicode},'download decrypt');
        const cache=JSON.parse($substore.read('#sub-store-cached-resource'));
        equal(cache[ProxyUtils.hex_md5('clash.meta/v1.19.23'+url)].data,pair.items[1].armor,'cache retains ciphertext');
        equal(await ProxyUtils.download(url+'#age-secret-key='+encodeURIComponent(pair.secret)),texts.unicode,'fragment decrypts cache');
      }
      const url='$UPSTREAM/host/download/age/wrong-key';
      await reject(()=>ProxyUtils.download(url+'#age-secret-key='+pairs[1].secret),error=>error.message.includes('age 解密失败:')&&!error.message.includes(pairs[1].secret));
      equal(await ProxyUtils.download(url,undefined,undefined,undefined,undefined,undefined,undefined,false,{'age-secret-key':pairs[0].secret}),texts.unicode,'download recovery');
      return {types:2,cacheContainsCiphertext:true,secretRedacted:true,recovered:true};
    });
  } finally {globalThis.Date=OriginalDate;}
  proxies[0].ageVerified=passed;proxies[0].ageResults=results;
  return proxies;
}
