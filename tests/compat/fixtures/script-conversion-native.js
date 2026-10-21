async function operator(proxies) {
  const assert=(value,message)=>{if(!value)throw new Error(message);};
  const equal=(actual,expected,message)=>assert(JSON.stringify(actual)===JSON.stringify(expected),message+': '+JSON.stringify({actual,expected}));
  const passed=[];
  globalThis.probeReenter=async(payload,callback)=>{await $substore.wait(1);return callback(payload);};
  try {
    eval($arguments.tests);
    await globalThis.runConversionContracts(async(id,callback)=>{await callback();passed.push(id);}, {ProxyUtils},
      (content,input,args)=>ProxyUtils.process(input,[{type:'Script Operator',args:{content,arguments:args}}]), assert,equal);
    const typed=ProxyUtils.parse('{type:"ss",name:"typed",server:"typed.invalid",port:443,extra:{nil:NaN,high:Infinity,low:-Infinity,minusZero:-0}}')[0];
    assert(Number.isNaN(typed.extra.nil) && typed.extra.high===Infinity && typed.extra.low===-Infinity && Object.is(typed.extra.minusZero,-0),'parse retains nonfinite numbers and negative zero');
    equal(JSON.parse(ProxyUtils.produce([typed],'JSON'))[0].extra,{nil:null,high:null,low:null,minusZero:0},'JSON wire numbers');
    const originalJSON5Parse=SubStoreUtilities.JSON5.parse;
    try {
      SubStoreUtilities.JSON5.parse=()=>{throw new Error('application utility was used by native parser');};
      const isolated=ProxyUtils.parse("{type:'ss',name:'isolated',server:'127.0.0.1',port:443,cipher:'none',password:'p'}");
      equal(isolated.map(proxy=>proxy.name),['isolated'],'reused parser remains isolated from application utility changes');
    } finally {
      SubStoreUtilities.JSON5.parse=originalJSON5Parse;
    }
    passed.push('NC-native-typed-numbers-and-wire');
    const cycle={name:'cycle',type:'ss',server:'cycle.invalid',port:443};cycle.extra={self:null};cycle.extra.self=cycle.extra;
    const internal=ProxyUtils.produce([cycle],'JSON','internal');
    assert(internal[0]===cycle && internal[0].extra.self===cycle.extra,'internal cyclic object identity');
    let rejected=false;try{ProxyUtils.produce([cycle],'JSON');}catch(error){rejected=error instanceof TypeError;}assert(rejected,'cyclic JSON rejects');
    passed.push('NC-native-json-cycle-and-error');
    const shadow=ProxyUtils.parse('ss://none:p@127.0.0.1:443?plugin=shadow-tls%3Bversion%3Dbad')[0];
    assert(Number.isNaN(shadow['plugin-opts'].version),'invalid numeric URI field remains NaN');
    const snell=ProxyUtils.parse('Huge = snell, 127.0.0.1, 443, psk=p, version='+'9'.repeat(400))[0];
    assert(snell.version===Infinity,'overflowed client numeric field remains Infinity');
    passed.push('NC-native-client-nonfinite-fields');
    const parsedCycle=ProxyUtils.parse('{type: ss, name: cyclic, server: 127.0.0.1, port: 443, cipher: none, password: p, extra: &loop {self: *loop}}')[0];
    assert(parsedCycle.extra.self===parsedCycle.extra,'YAML cyclic field survives native parse');
    assert(ProxyUtils.produce([parsedCycle],'URI').startsWith('ss://'),'URI ignores cyclic extra fields');
    let cycleMessage='';try{ProxyUtils.produce([parsedCycle],'JSON');}catch(error){cycleMessage=error.message;}
    assert(cycleMessage.includes("property 'self' closes the circle"),'JSON cyclic error identifies the back reference');
    passed.push('NC-native-yaml-cycles-and-unused-fields');
    const lone=String.fromCharCode(0xd800),low=String.fromCharCode(0xdfff);
    const literal=String.fromCodePoint(0xf0000)+'0'+String.fromCodePoint(0xf0001);
    const unpaired=ProxyUtils.parse('{type:"ss",name:"'+lone+'",server:"127.0.0.1",port:443,cipher:"none",password:"p",extra:{"'+low+'":"'+literal+'"}}')[0];
    assert(unpaired.name===lone && unpaired.extra[low]===literal,'isolated UTF16 fields and keys survive parse without tag collisions');
    assert(JSON.parse(ProxyUtils.produce([unpaired],'JSON'))[0].name===lone,'isolated UTF16 survives JSON wire');
    passed.push('NC-native-isolated-utf16-input-and-wire');
    for(const headers of [["a",2],"raw",true,7,null]) {
      const proxy=ProxyUtils.parse('vless://id@127.0.0.1:443?type=ws&obfsParam='+encodeURIComponent(JSON.stringify(headers))+'#headers')[0];
      equal(proxy['ws-opts'].headers,headers,'non-object transport headers retain their type');
    }
    passed.push('NC-native-transport-header-values');
    const nested=ProxyUtils.parse('trojan://p@127.0.0.1:443?type=transport%5B0%5D.ws&path=%2Fp&host=example.com#nested')[0];
    equal(nested.transport,[{'ws-opts':{path:'/p',headers:{Host:'example.com'}}}],'nested Trojan transport paths and array creation');
    equal(ProxyUtils.parse('trojan://p@127.0.0.1:443?type=__proto__.bad&path=%2Fp'),[],'unsafe transport path is rejected');
    passed.push('NC-native-nested-trojan-paths');
    const caPath=$arguments.caFile;
    const certificate=ProxyUtils.parse(JSON.stringify({type:'trojan',name:'file-ca',server:'127.0.0.1',port:443,password:'p',_ca:caPath}))[0];
    equal(certificate['tls-fingerprint'],'05:4E:DE:C1:D0:21:1F:62:4F:ED:0C:BC:A9:D4:F9:40:0B:0E:49:1C:43:74:2A:F2:C5:B0:AB:EB:F0:C9:90:D8','CA file fingerprint');
    assert(certificate._ca===caPath && !Object.hasOwn(certificate,'ca-str'),'CA path remains data');
    const missing=ProxyUtils.parse(JSON.stringify({type:'trojan',name:'missing-ca',server:'127.0.0.1',port:443,password:'p',_ca:caPath+'.missing'}))[0];
    assert(!missing['tls-fingerprint'],'missing CA file does not stop parsing');
    passed.push('NC-native-ca-file-and-read-failure');
  } catch(error) {
    return proxies.map(proxy=>({...proxy,conversionHelpersError:String(error),conversionHelpersPassed:passed}));
  }
  $substore.write({passed},'m3-native-conversion-helpers');
  return proxies.map(proxy=>({...proxy,conversionHelpersVerified:passed.length}));
}
