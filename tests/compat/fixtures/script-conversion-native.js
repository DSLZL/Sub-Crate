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
  } catch(error) {
    return proxies.map(proxy=>({...proxy,conversionHelpersError:String(error),conversionHelpersPassed:passed}));
  }
  $substore.write({passed},'m3-native-conversion-helpers');
  return proxies.map(proxy=>({...proxy,conversionHelpersVerified:passed.length}));
}
