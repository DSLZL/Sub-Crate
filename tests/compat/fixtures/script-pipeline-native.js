async function operator(proxies) {
  const assert=(value,message)=>{if(!value)throw new Error(message);};
  const equal=(actual,expected,message)=>assert(JSON.stringify(actual)===JSON.stringify(expected),message+': '+JSON.stringify({actual,expected}));
  const rejects=async(callback,message)=>{try{await callback();}catch(error){assert(error.message.includes(message),'pipeline error '+error.message);return;}throw new Error('expected '+message);};
  const DateBefore=Date,subsBefore=$substore.read('subs'),colsBefore=$substore.read('collections');
  globalThis.Date=class extends DateBefore {constructor(...args){super(...(args.length?args:[3300000000000]));}static now(){return 3300000000000;}};
  const passed=[],results={};
  const check=async(id,callback)=>{results[id]=await callback();passed.push(id);};
  const local='ss://YWVzLTEyOC1nY206cGFzcw==@local.invalid:8388#local';
  const remote='$UPSTREAM/host/pipeline/';
  const names=value=>value.map(item=>item.name);
  const produce=options=>produceArtifact({type:'subscription',platform:'JSON',produceType:'internal',noFlow:true,...options});
  const source={name:'源🙂',displayName:'显示',source:'local',content:local};
  try {
    await check('PL-native-subscription-source-precedence-and-typed-output',async()=>{
      const result=await produce({subscription:source});
      equal(names(result),['local'],'local source');
      equal([result[0]._subName,result[0]._subDisplayName],['源🙂','显示'],'provenance');
      const overridden=await produce({subscription:{...source,source:'remote',url:remote+'a'},content:local});
      equal(names(overridden),['local'],'content override bypasses URL');
      const raw=await produce({subscription:source,produceType:'raw',includeRaw:true});
      equal(raw,{result:JSON.stringify([local]),raw:[local]},'raw output and original source');
      await rejects(()=>produce({name:'missing'}),'找不到订阅 missing');
      await rejects(()=>produce({subscription:undefined}),'未提供订阅名称或订阅数据');
      return {names:names(result),raw,missing:true};
    });
    await check('PL-native-parallel-source-order-merge-and-raw',async()=>{
      const sub={...source,source:'remote',url:remote+'slow\n'+remote+'b',mergeSources:'localFirst'};
      const result=await produce({subscription:sub,includeRaw:true});
      equal(names(result.result),['local','remote-slow','remote-b'],'source order survives completion order');
      equal(result.raw,[local,'ss://YWVzLTEyOC1nY206cGFzcw==@slow.invalid:8388#remote-slow','ss://YWVzLTEyOC1nY206cGFzcw==@b.invalid:8388#remote-b'],'original raw sources');
      equal(names(await produce({subscription:{...sub,mergeSources:'remoteFirst'}})),['remote-slow','remote-b','local'],'remoteFirst and shared cache');
      const override=await produce({subscription:source,url:remote+'a',content:local,mergeSources:'remoteFirst'});
      equal(names(override),['remote-a','local'],'overrides own source and merge');
      return {result,override};
    });
    await check('PL-native-processing-options-source-and-original-content',async()=>{
      const sub={...source,process:[{type:'Script Operator',args:{content:`async function operator(items,target,context){
        if(target!=='JSON'||$options.zero!==0||context.source['源🙂'].displayName!=='显示')throw new Error('pipeline injection');
        items[0].raw=context.raw;items[0].name+='-processed';return items;}`}}]};
      const result=await produce({subscription:sub,$options:{zero:0}});
      equal(result[0].raw,[local],'raw injected unchanged');
      equal(names(result),['local-processed'],'processor used before producer');
      return result;
    });
    await check('PL-native-collection-explicit-tags-child-and-own-order',async()=>{
      const first={...source,name:'first',tag:['自动'],process:[{type:'Regex Rename Operator',args:[{expr:'local',now:'child-first'}]}]};
      const second={name:'second',source:'remote',url:remote+'a',tag:['自动']};
      const excluded={...source,name:'excluded',tag:['其他']};
      const collection={name:'组合🙂',displayName:'集合显示',subscriptions:['first'],subscriptionTags:['自动'],process:[{type:'Script Operator',args:{content:`function operator(items,target,context){
        if($options.zero!==0||context.source._collection.name!=='组合🙂')throw new Error('collection injection');
        items.forEach(item=>{item.collectionRaw=context.raw;item.name+='-collection';});return items;}`}}]};
      $substore.write([first,second,excluded],'subs');$substore.write([collection],'collections');
      const result=await produce({type:'collection',name:'组合🙂',$options:{zero:0},includeRaw:true});
      equal(names(result.result),['child-first-collection','remote-a-collection'],'explicit then tagged source and own processing');
      equal(result.result.map(item=>[item._subName,item._collectionName,item._collectionDisplayName]),[['first','组合🙂','集合显示'],['second','组合🙂','集合显示']],'collection provenance');
      equal(Object.keys(result.raw),['first','second'],'raw map keys');
      await rejects(()=>produce({type:'collection',name:'missing'}),'找不到组合订阅 missing');
      return result;
    });
    await check('PL-native-source-ignore-and-subscription-fallback',async()=>{
      const sub={...source,source:'remote',url:remote+'failed\n'+remote+'a'};
      await rejects(()=>produce({subscription:sub}),'发生错误, 请查看日志');
      equal(names(await produce({subscription:{...sub,ignoreFailedRemoteSub:true}})),['remote-a'],'true means quiet partial output');
      equal(await produce({subscription:{...sub,ignoreFailedRemoteSub:'fallbackQuiet'}}),[],'fallback discards partially loaded source');
      equal(await produce({subscription:{...source,content:'',ignoreFailedRemoteSub:'fallbackNotify'}}),[],'empty source fallback');
      equal(await produce({subscription:{...sub,ignoreFailedRemoteSub:'fallbackQuiet'},produceType:'raw',includeRaw:true}),{result:'[]',raw:[]},'raw fallback retains source array at failure boundary');
      return {partial:true,wholeFallback:true,rawFallback:true};
    });
    await check('PL-native-collection-partial-and-whole-fallback',async()=>{
      const valid={...source,name:'valid'},invalid={name:'invalid',source:'remote',url:remote+'failed'};
      $substore.write([valid,invalid],'subs');
      const collection={name:'fallback',subscriptions:['invalid','valid'],ignoreFailedRemoteSub:'fallbackQuiet'};
      $substore.write([collection],'collections');
      const result=await produce({type:'collection',name:'fallback',includeRaw:true});
      equal(names(result.result),['local'],'collection retains successful children');
      equal(result.raw.invalid,[],'failed child raw replaced by empty array');
      $substore.write([{...collection,subscriptions:['invalid']}],'collections');
      equal(await produce({type:'collection',name:'fallback'}),[],'no success falls back whole collection');
      $substore.write([{...invalid,ignoreFailedRemoteSub:'fallbackQuiet'}],'subs');
      $substore.write([{name:'child-fallback',subscriptions:['invalid']}],'collections');
      await rejects(()=>produce({type:'collection',name:'child-fallback'}),'不含有效节点');
      return {result,empty:true,childFallback:true};
    });
    proxies[0].pipelineVerified=passed;proxies[0].pipelineResults=results;return proxies;
  } finally {$substore.write(subsBefore,'subs');$substore.write(colsBefore,'collections');globalThis.Date=DateBefore;}
}
