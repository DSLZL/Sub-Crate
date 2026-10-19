async function operator(proxies) {
  const assert=(value,message)=>{if(!value)throw new Error(message);};
  const equal=(actual,expected,message)=>assert(JSON.stringify(actual)===JSON.stringify(expected),message+': '+JSON.stringify({actual,expected}));
  globalThis.probeUpstream='$UPSTREAM';
  const passed=[];
  try {
    const run=eval($arguments.tests+'\nrunGistContracts');
    await run(async(id,callback)=>{await callback();passed.push(id);},{ProxyUtils},
      (content,input,args)=>ProxyUtils.process(input,[{type:'Script Operator',args:{content,arguments:args}}]),assert,equal);
  } catch(error) {
    return proxies.map(proxy=>({...proxy,gistError:String(error),gistPassed:passed}));
  }
  $substore.write({passed},'m3-native-gist');
  return proxies.map(proxy=>({...proxy,gistVerified:passed.length}));
}
