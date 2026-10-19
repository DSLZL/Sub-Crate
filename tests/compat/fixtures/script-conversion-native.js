async function operator(proxies) {
  const assert=(value,message)=>{if(!value)throw new Error(message);};
  const equal=(actual,expected,message)=>assert(JSON.stringify(actual)===JSON.stringify(expected),message+': '+JSON.stringify({actual,expected}));
  const passed=[];
  globalThis.probeReenter=async(payload,callback)=>{await $substore.wait(1);return callback(payload);};
  eval($arguments.tests);
  await globalThis.runConversionContracts(async(id,callback)=>{await callback();passed.push(id);}, {ProxyUtils},
    (content,input,args)=>ProxyUtils.process(input,[{type:'Script Operator',args:{content,arguments:args}}]), assert,equal);
  $substore.write({passed},'m3-native-conversion-helpers');
  return proxies.map(proxy=>({...proxy,conversionHelpersVerified:passed.length}));
}
