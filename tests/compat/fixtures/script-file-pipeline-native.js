async function operator(proxies) {
  const assert=(value,message)=>{if(!value)throw new Error(message);};
  const equal=(actual,expected,message)=>assert(JSON.stringify(actual)===JSON.stringify(expected),message+': '+JSON.stringify({actual,expected}));
  const rejects=async(callback,message)=>{try{await callback();}catch(error){assert(error.message.includes(message),'file error '+error.message);return;}throw new Error('expected '+message);};
  const savedFiles=$substore.read('files'),savedSubs=$substore.read('subs'),DateBefore=Date;
  globalThis.Date=class extends DateBefore {constructor(...args){super(...(args.length?args:[3400000000000]));}static now(){return 3400000000000;}};
  const passed=[],results={};
  const check=async(id,callback)=>{results[id]=await callback();passed.push(id);};
  const produce=options=>produceArtifact({type:'file',noFlow:true,...options});
  const local='ss://YWVzLTEyOC1nY206cGFzcw==@file.invalid:8388#file-node';
  const yaml=ProxyUtils.yaml;
  try {
    await check('FL-native-local-raw-array-join-and-output-fields',async()=>{
      const file={name:'text',source:'local',content:[['a',null],0,false,'']};
      equal(await produce({file}),'a\n0\nfalse','one-level flatten and join');
      equal(await produce({file,produceType:'raw'}),JSON.stringify(['a',null,0,false,'']),'raw before processing');
      const output=await produce({file,all:true});
      equal(output,{$content:'a\n0\nfalse',$files:['a',null,0,false,'']},'unprocessed file fields');
      assert(!Object.hasOwn(output,'$file'),'unprocessed output omits file');
      equal(await produce({file,content:'override'}),'override','content override');
      await rejects(()=>produce({name:'missing'}),'找不到文件 missing');
      await rejects(()=>produce({file:{name:'missing-url',source:'remote',url:''}}),'未配置远程文件 URL');
      return output;
    });
    await check('FL-native-full-shortcut-options-and-unknown-fields',async()=>{
      const file={name:'script',source:'local',content:'base',unknown:{zero:0,flag:false},process:[{type:'Script Operator',args:{content:`$content += '-shortcut'; if($options.zero!==0||$file.unknown.flag!==false)throw new Error('file bindings');`}},
        {type:'Script Operator',args:{content:'async function operator(input){await $substore.wait(1);input.$content+="-full";return input;}'}}]};
      const output=await produce({file,all:true,$options:{zero:0}});
      equal(output.$content,'base-shortcut-full','both script entries');
      equal([output.$options,output.$file.unknown],[{zero:0},{zero:0,flag:false}],'local file options');
      return output;
    });
    await check('FL-native-generated-file-script-loading-and-arguments',async()=>{
      const file={name:'script-native',source:'local',content:'function operator(items){items[0].name += $arguments.suffix;return items;}'};
      $substore.write([file],'files');
      const result=await ProxyUtils.process([{name:'base'}],[{type:'Script Operator',args:{mode:'link',content:'/api/file/script-native#suffix='+encodeURIComponent('-文件🙂')}}]);
      equal(result,[{name:'base-文件🙂'}],'generated file as source');
      return result;
    });
    await check('FL-native-mihomo-source-and-legacy-type',async()=>{
      const file={name:'mihomo',type:'mihomoProfile',sourceType:'local',mode:'proxy',content:local};
      const output=await produce({file});
      const parsed=yaml.safeLoad(output);
      equal(parsed.proxies.map(item=>item.name),['file-node'],'proxy-only mode');
      assert(!Object.keys(parsed.proxies[0]).some(key=>key.startsWith('_')),'generated metadata removed');
      equal(await produce({file:{...file,sourceType:'none'}}),'{}\n','none source');
      const processed=await produce({file:{...file,mode:'text',content:'keep: true\n',process:[{type:'Script Operator',args:{content:'function operator(input){input.$content+="type: "+input.$file.type+"\\n";return input;}'}}]},all:true});
      equal(yaml.safeLoad(processed.$content),{keep:true,type:'mihomoConfig'},'legacy type normalized before processing');
      return {parsed,processed};
    });
    await check('FL-native-mihomo-yaml-patch-main-and-short-id',async()=>{
      const file={name:'patch',type:'mihomoConfig',sourceType:'local',content:'items: [old]\nnested: {keep: {old: true}, zero: 1}\n'};
      const patch='nested:\n  keep!: {fresh: true}\n  zero: 0\n+items: [front]\nitems+: [back]\n<literal>: false\n';
      const output=await produce({file:{...file,process:[{type:'Script Operator',args:{content:patch}}]}});
      equal(yaml.safeLoad(output),{items:['front','old','back'],nested:{keep:{fresh:true},zero:0},'<literal>':false},'patch recursion and prepend/append');
      const main=await produce({file:{...file,process:[{type:'Script Operator',args:{content:'async function main(config){await $substore.wait(1);config.main=false;return config;}'}}]}});
      equal(yaml.safeLoad(main),{items:['old'],nested:{keep:{old:true},zero:1},main:false},'async main');
      const reality='proxies:\n  - {name: node, type: vless, reality-opts: {short-id: 0088}}\n';
      const normalized=await produce({file:{...file,content:reality}});
      assert(normalized.includes('short-id: "0088"'),'short ID keeps leading zeroes');
      return {output,main,normalized};
    });
    await check('FL-native-add-subscription-proxies-positions-and-skip',async()=>{
      $substore.write([{name:'add-native',source:'local',content:local}],'subs');
      const file={name:'add',type:'mihomoConfig',sourceType:'local',content:'proxies: [{name: existing, type: direct}]\n'};
      const outputs={};
      for(const position of ['front','back','replace']) {
        const output=await produce({file:{...file,process:[{type:'Add Proxies From Subscription Operator',args:{sourceName:'add-native',position}}]}});
        outputs[position]=yaml.safeLoad(output).proxies.map(item=>item.name);
      }
      equal(outputs,{front:['file-node','existing'],back:['existing','file-node'],replace:['file-node']},'Add positions');
      equal(await ProxyUtils.process([{name:'unchanged'}],[{type:'Add Proxies From Subscription Operator',args:{sourceName:'missing'}}]),[{name:'unchanged'}],'non-file input skips generator');
      return outputs;
    });
    proxies[0].filePipelineVerified=passed;proxies[0].filePipelineResults=results;return proxies;
  } finally {$substore.write(savedFiles,'files');$substore.write(savedSubs,'subs');globalThis.Date=DateBefore;}
}
