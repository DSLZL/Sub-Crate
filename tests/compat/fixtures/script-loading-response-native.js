async function operator(proxies) {
  const passed=[], observations={};
  const assert=(value,message)=>{if(!value)throw new Error(message);};
  const equal=(a,b,message)=>assert(JSON.stringify(a)===JSON.stringify(b),message);
  const check=async(id,callback)=>{observations[id]=await callback();passed.push(id);};
  const rejects=async(callback,message)=>{try{await callback();}catch(error){assert(error.message.includes(message),'loader/response error: '+error.message);return;}throw new Error('expected error');};
  const OriginalDate=Date;
  globalThis.Date=class extends OriginalDate {
    constructor(...args){super(...(args.length?args:[3200000000000]));}
    static now(){return 3200000000000;}
  };
  const modules=$substore.read('modules');
  const base='$UPSTREAM/host/script/';
  const source={marker:'source-marker'},options={zero:0,flag:false};
  const link=(content,type='Script Operator')=>({type,args:{mode:'link',content}});
  const process=(items,execution={})=>ProxyUtils.process([{name:'linked',server:'example.invalid',port:443}],items,'JSON',source,options,false,execution);
  const transformed=(input,items,execution={})=>ProxyUtils.processResponse(input,items,'JSON',source,options,execution);
  const transform=content=>({type:'Response Transformer',args:{content}});
  try {
    await check('SC-native-linked-remote-arguments-cache-and-two-fragments',async()=>{
      const args={zero:0,flag:false,text:'链接🙂'};
      let result=await process([link(base+'operator#'+encodeURIComponent(JSON.stringify(args)))]);
      equal(result[0].linked,{arguments:args,target:'JSON',options,raw:[false],source:'source-marker'},'full linked injection');
      equal((await process([link(base+'operator#zero=0&flag=false&empty=&enabled')]))[0].linked.arguments,
        {zero:'0',flag:'false',empty:true,enabled:true},'paired arguments share the cached source');
      const headers=JSON.stringify({'User-Agent':'linked-script-native','X-Script-Token':'two-fragments'});
      result=await process([link(base+'headers#'+encodeURIComponent(JSON.stringify(args))+'#headers='+encodeURIComponent(headers))]);
      equal(result[0].linked.arguments,args,'third fragment only controls download');
      return {jsonArguments:true,pairArguments:true,sourceCached:true,thirdFragment:true};
    });
    await check('SC-native-linked-no-cache-and-execution-context',async()=>{
      await process([link(base+'nocache#noCache')]);
      await process([link(base+'nocache#noCache')]);
      await process([link(base+'flow#zero=0#validCheck&noCache')],{noFlow:true});
      await rejects(()=>process([link(base+'flow#zero=0#validCheck&noCache')]),'无法下载脚本:');
      return {refreshRequests:2,noFlowPropagates:true,flowError:true};
    });
    await check('SC-native-linked-modules-filter-and-errors',async()=>{
      $substore.write([{name:'模块🙂',content:'function operator(proxies) { proxies[0].module=$arguments.text; return proxies; }'},
        {name:'过滤',content:'return $server.port===443;'}],'modules');
      equal((await process([link('/api/module/'+encodeURIComponent('模块🙂')+'#text='+encodeURIComponent('内容🙂'))]))[0].module,'内容🙂','module source and args');
      equal((await process([link('/api/module/'+encodeURIComponent('过滤'),'Script Filter')])).length,1,'linked shortcut filter');
      await rejects(()=>process([link('/api/module/missing#x=1')]),'无法加载 module: /api/module/missing');
      await rejects(()=>process([link(base+'missing')]),'无法下载脚本:');
      await process([link(base+'unknown','Unknown Script Operator')]);
      return {modules:true,filter:true,missing:true,unknownStillLoads:true};
    });
    await check('RS-native-normalization-aliases-and-full-transform',async()=>{
      const input={statusCode:201,headers:{before:'yes'},body:'响应🙂\r\n',unknown:false};
      const output=await transformed(input,[transform(`async function transformFunction(res,context) {
        if(context.source.marker!=='source-marker'||$options.zero!==0)throw new Error('response injection');
        if(res.header!==res.headers)throw new Error('headers are aliases');
        await $substore.wait(1);res.header={changed:false};res.headers.zero=0;res.body+='after';return res;
      }`)]);
      equal(output,{status:201,body:'响应🙂\r\nafter',headers:{changed:false,zero:0},header:{changed:false,zero:0}},'normalized full response');
      assert(output.header===output.headers&&!Object.hasOwn(output,'unknown'),'aliases and selected fields');
      equal(input.headers,{before:'yes'},'replaced aliases retain original header object');
      output.header=null;assert(output.header===output.headers&&Object.keys(output.headers).length===0,'invalid alias setter');
      return {status:201,body:'响应🙂\r\nafter',aliases:true,unknownDropped:true};
    });
    await check('RS-native-shortcut-http-and-falsy-results',async()=>{
      const output=await transformed({status:202,body:'响应🙂\r\n',header:{}},[transform(`const response=await $substore.http.post({url:'$UPSTREAM/host/echo',body:$res.body,headers:{'X-Probe':'native-response'}});
        $res.headers.echo=JSON.parse(response.body).body;$res.body+='shortcut';`)]);
      equal(output,{status:202,body:'响应🙂\r\nshortcut',headers:{echo:'响应🙂\r\n'},header:{echo:'响应🙂\r\n'}},'shortcut native HTTP');
      for(const value of ['undefined','null','false','0','""']) {
        const result=await transformed({body:'start'},[transform('function transformFunction(res){res.body="mutated";return '+value+';}')]);
        equal(result.body,'mutated','falsy result retains mutation');
      }
      equal(await transformed({status:0,statusCode:203,header:false,headers:{zero:0}},[]),
        {status:203,body:'',headers:{zero:0},header:{zero:0}},'truthy precedence and missing body');
      return {nativeHttp:true,falsyResults:5,precedence:true};
    });
    await check('RS-native-control-linked-transform-errors-and-recovery',async()=>{
      const output=await transformed({body:'a'},[
        {type:'Script Operator',args:{content:'throw new Error("wrong stage");'}},
        {...transform('throw new Error("disabled");'),disabled:true},
        transform('function transformFunction(res,context){context.process={type:"disable",customNames:["skip"]};res.body+="b";return res;}'),
        {...transform('throw new Error("skipped");'),customName:'skip'},
        link(base+'response#text='+encodeURIComponent('链接🙂'),'Response Transformer'),
      ]);
      equal(output.body,'ab链接🙂','stage skipping and linked transform');
      await rejects(()=>transformed({body:''},[transform('throw new Error("response-sentinel");')]),'响应修改 执行失败 response-sentinel');
      equal((await transformed({body:'recovered'},[])).body,'recovered','response error recovery');
      return {controls:true,linked:true,error:true,recovery:true};
    });
    proxies[0].loadingResponseVerified=passed;proxies[0].loadingResponseResults=observations;
    return proxies;
  } finally {$substore.write(modules,'modules');globalThis.Date=OriginalDate;}
}
