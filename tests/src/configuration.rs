use crate::{Result,api::{self,Api},json,process,raw_http,reference,require,server::{self,Backend},service,sharing,static_files,upstream::Upstream};
use base64::{Engine,engine::general_purpose::STANDARD};
use serde_json::{Value,json};
use std::{collections::BTreeMap,path::Path,process::Command,time::Duration};

pub fn run(output:&Path)->Result{
    reference::verify(output)?;let(baseline,root)=service::baseline(output)?;
    for(mode,configured,maximum)in[("default",None,32768),("custom",Some(" +65536junk"),65536),("fallback",Some("0"),32768),("small",Some("512"),512)]{
        for(kind,backend)in[("node",Backend::Node(reference::node())),("rust",service::target()?)]{
            let label=format!("{kind}-runtime-config-{mode}");let mut document=baseline.clone();document["settings"]=json!({});for field in["subs","collections","files","artifacts","tokens","archives","modules"]{document[field]=json!([]);}
            let(state,mut env)=service::seed(output,&label,&document,&root)?;if let Some(configured)=configured{server::set(&mut env,"SUB_STORE_MAX_HEADER_SIZE",configured);}if mode=="custom"{server::set(&mut env,"SUB_STORE_BACKEND_DEFAULT_PROXY","unsupported://fixture");}
            let upstream=Upstream::new(0)?;let api=Api::start(&backend,&env,&state,output,&label)?;
            let script=format!("async function operator(proxies){{const result=[];for(const size of [16000,40000,70000]){{try{{const response=await $substore.http.get({base}+'/host/header-size/'+size);result.push({{size,body:response.body}});}}catch(error){{result.push({{size,error:error.message}});}}}}let blocked=false;try{{await ProxyUtils.download({base}+'/host/header-size/100');}}catch(error){{blocked=error instanceof Error;}}result.push({{defaultProxyBlocked:blocked}});$substore.write(JSON.stringify(result),'#m5-runtime-configuration');return proxies;}}",base=json::quote(&upstream.url()));
            api.send(&json!({"method":"POST","path":"/api/preview/sub?target=JSON","body":{"name":label,"source":"local","content":"ss://YWVzLTEyOC1nY206cGFzcw==@127.0.0.1:8388#fixture","process":[{"type":"Script Operator","args":{"content":script}}]}}),200)?;
            api.send(&json!({"path":"/api/settings"}),200)?;
            let mut expected=[16000,40000,70000].into_iter().map(|size|if size<maximum{json!({"size":size,"body":"header-fixture"})}else{json!({"size":size,"error":"Headers Overflow Error"})}).collect::<Vec<_>>();expected.push(json!({"defaultProxyBlocked":mode=="custom"}));
            let saved=api.stored("root.json")?;json::check(&json::parse(saved["m5-runtime-configuration"].as_str().ok_or("Missing script header-limit proof")?)?,&json!(expected),false)?;
            let requests=upstream.requests();require(requests.iter().any(|request|request["path"]=="/host/header-size/100")== (mode!="custom"),"Default proxy was ignored or direct download failed")?;
            json::write_new(output.join(format!("{label}.json")),&json!({"maximum":maximum,"configured":configured,"expected":expected,"root":saved,"upstream":requests}))?;upstream.fixture.healthy()?;
        }
    }
    let configurations=json!([
        ["default",{},1048576,[["https://sub-store.vercel.app","https://sub-store.vercel.app"]],"https://denied.invalid",false],
        ["custom-name",{"SUB_STORE_BACKEND_CUSTOM_NAME":"configuration-fixture","SUB_STORE_BODY_JSON_LIMIT":"+1GB","SUB_STORE_X_POWERED_BY":"m5-configuration"},1073741824,[["http://127.0.0.1:9911","http://127.0.0.1:9911"]],"https://127.0.0.1:9911",false],
        ["wildcard",{"SUB_STORE_CORS_ALLOWED_ORIGINS":"https://unused.invalid,*","SUB_STORE_BODY_JSON_LIMIT":"1.5 kb"},1536,[["https://arbitrary.invalid","https://arbitrary.invalid"]],null,true],
        ["zero",{"SUB_STORE_BODY_JSON_LIMIT":"0","SUB_STORE_CORS_ALLOWED_ORIGINS":"'https://ALLOWED.invalid/a'，https://second.invalid"},0,[["https://allowed.invalid","https://allowed.invalid"],["https://second.invalid","https://second.invalid"]],"https://denied.invalid",false],
        ["integer-prefix",{"SUB_STORE_BODY_JSON_LIMIT":"64junk","SUB_STORE_BACKEND_PREFIX":"false","SUB_STORE_FRONTEND_BACKEND_PATH":"/custom","SUB_STORE_CORS_ALLOWED_ORIGINS":"\u{feff}https://allowed.invalid\u{feff}"},64,[["https://allowed.invalid","https://allowed.invalid"]],"https://denied.invalid",false],
        ["integer-exponent",{"SUB_STORE_BODY_JSON_LIMIT":"1e3"},1,[["https://sub-store.vercel.app","https://sub-store.vercel.app"]],"https://denied.invalid",false]
    ]);
    for row in configurations.as_array().unwrap(){
        let mut paired=Vec::new();
        for(kind,backend)in[("node",Backend::Node(reference::node())),("rust",service::target()?)]{
            let label=format!("{kind}-config-{}",row[0].as_str().unwrap());let mut root=root.clone();root["configurationRoot"]=json!([null,false,0]);let(state,mut env)=service::seed(output,&label,&baseline,&root)?;server::set(&mut env,"NODE_ENV","production");
            for(key,value)in row[1].as_object().unwrap(){server::set(&mut env,key,value.as_str().unwrap());}
            let api=Api::start(&backend,&env,&state,output,&label)?;let mut observed=Vec::new();
            let mut send=|step:Value,status:u16|->Result<Value>{
                let before=api::files(&api)?;let response=api.response(&step)?;require(response["status"]==status,format!("Listener configuration response changed: {label} {step} {response}"))?;
                if status>=400||["GET","OPTIONS"].contains(&step["method"].as_str().unwrap_or("GET")){require(api::files(&api)?==before,"Configuration rejection/read changed files")?;}
                observed.push(json!({"request":step,"response":{"status":response["status"],"body":response["body"]},"headers":{"content-type":response["headers"]["content-type"],"access-control-allow-origin":response["headers"]["access-control-allow-origin"],"vary":response["headers"]["vary"],"x-powered-by":response["headers"]["x-powered-by"]},"state":api.durable()?,"root":api.stored("root.json")?}));Ok(response)
            };
            let wildcard=row[5]==true;let response=send(json!({"path":"/api/settings"}),200)?;
            require(response["headers"]["x-powered-by"]==row[1].get("SUB_STORE_X_POWERED_BY").cloned().unwrap_or(json!("Sub-Store"))&&response["headers"]["access-control-allow-origin"]==if wildcard{json!("*")}else{Value::Null},"Default configuration headers changed")?;
            for origins in row[3].as_array().unwrap(){
                let response=send(json!({"path":"/api/settings","headers":{"Origin":origins[0]}}),200)?;
                require(response["headers"]["access-control-allow-origin"]==if wildcard{json!("*")}else{origins[1].clone()},"Allowed CORS origin normalization changed")?;
                send(json!({"path":"/not-a-route","method":"OPTIONS","headers":{"Origin":origins[0]},"body":{"configProbe":"x".repeat(2000)}}),200)?;
            }
            if !row[4].is_null(){let response=send(json!({"path":"/api/settings","method":"PATCH","headers":{"Origin":row[4]},"body":{"configProbe":"x".repeat(2000)}}),403)?;require(response["body"]=="CORS origin not allowed"&&response["headers"].get("access-control-allow-origin").is_none(),"Denied CORS origin gained access")?;}
            for value in[json!(1),json!("x".repeat(2000))]{let body=json!({"configProbe":value});let status=if json::stringify(&body).len()<=row[2].as_u64().unwrap() as usize{200}else{413};send(json!({"path":"/api/settings","method":"PATCH","body":body}),status)?;}
            send(json!({"path":"/api/settings","origin":true}),404)?;json::write_new(output.join(format!("{label}.json")),&json!(observed))?;paired.push(observed);
        }json::check(&json!(paired[0]),&json!(paired[1]),false)?;
    }
    Ok(())
}

pub fn diagnostics(output:&Path)->Result{
    reference::verify(output)?;let(baseline,root)=service::baseline(output)?;
    let mut cases=vec![
        ("missing-subscription","GET","/api/sub/missing",Vec::new(),json!({}),500,"TypeError: Cannot convert undefined or null to object",true),
        ("recipient-coercion","PATCH","/api/sub/diagnostic",br#"{"age-public-key":{"toString":null}}"#.to_vec(),json!({}),500,"TypeError: Cannot convert object to primitive value",true),
        ("module-invalid-status","GET","/api/module/bad-status",Vec::new(),json!({}),500,"RangeError [ERR_HTTP_INVALID_STATUS_CODE]: Invalid status code: 0",true),
        ("standalone-share-method","POST","/share/file/diagnostic?token=unused",Vec::new(),json!({}),500,"Error: Method not allowed",true),
        ("parameter-decoding","GET","/api/sub/%FF",Vec::new(),json!({}),400,"URIError: Failed to decode param '%FF'",true),
        ("static-precondition","GET","/chunks/main-84da2e86.js",Vec::new(),json!({"If-Match":"\"wrong\""}),412,"PreconditionFailedError: Precondition Failed",true),
        ("static-range","GET","/chunks/main-84da2e86.js",Vec::new(),json!({"Range":"bytes=99999-"}),416,"RangeNotSatisfiableError: Range Not Satisfiable",true),
        ("invalid-json","PATCH","/api/settings",b"{".to_vec(),json!({}),400,"SyntaxError:",false),
        ("scalar-json","PATCH","/api/settings",b"false".to_vec(),json!({}),400,"SyntaxError:",false),
        ("unsupported-charset","PATCH","/api/settings",b"{}".to_vec(),json!({"Content-Type":"application/json; charset=latin1"}),415,"UnsupportedMediaTypeError: unsupported charset \"LATIN1\"",true),
        ("unsupported-encoding","PATCH","/api/settings",b"{}".to_vec(),json!({"Content-Encoding":"br"}),415,"UnsupportedMediaTypeError: unsupported content encoding \"br\"",true),
        ("raw-verifier","PATCH","/api/settings",b"{}".to_vec(),json!({"Content-Type":"application/json; charset=utf-16be"}),403,"TypeError [ERR_UNKNOWN_ENCODING]: Unknown encoding: utf-16be",true),
        ("invalid-gzip","PATCH","/api/settings",b"not-gzip".to_vec(),json!({"Content-Encoding":"gzip"}),400,"Error:",false)
    ];cases.push(("json-limit","PATCH","/api/settings",format!("{{\"x\":\"{}\"}}","x".repeat(80)).into_bytes(),json!({}),413,"PayloadTooLargeError: request entity too large",true));
    for mode in["default","development","production"]{
        let mut paired=Vec::new();
        for(kind,backend)in[("node",Backend::Node(reference::node())),("rust",service::target()?)]{
            let label=format!("{kind}-diagnostics-{mode}");let mut document=baseline.clone();document["subs"]=json!([{"name":"diagnostic","source":"local","content":""}]);document["collections"]=json!([]);document["modules"]=json!([{"name":"bad-status","content":0}]);
            let(state,mut env)=service::seed(output,&label,&document,&root)?;static_files::prepare(&state.join("frontend"))?;
            server::set(&mut env,"SUB_STORE_FRONTEND_PATH",state.join("frontend"));server::set(&mut env,"SUB_STORE_FRONTEND_HOST","127.0.0.1");server::set(&mut env,"SUB_STORE_FRONTEND_PORT",server::free_port()?.to_string());server::set(&mut env,"SUB_STORE_BODY_JSON_LIMIT","64b");env.remove(std::ffi::OsStr::new("NODE_ENV"));if mode!="default"{server::set(&mut env,"NODE_ENV",mode);}
            let api=Api::start(&backend,&env,&state,output,&label)?;api::wait(||Ok(api.http.request(&api.listener.frontend,&json!({"path":"/"}))?["status"]==200))?;
            let mut observed=Vec::new();
            for(id,method,path,body,extra,status,detail,exact)in&cases{
                let before=api::files(&api)?;let public=path.starts_with("/chunks/")||path.starts_with("/share/");let mut headers=BTreeMap::from([("Content-Type".into(),"application/json".into())]);for(key,value)in extra.as_object().unwrap(){headers.insert(key.clone(),value.as_str().unwrap().into());}
                let reply=raw_http::request(if public{&api.listener.frontend}else{&api.listener.url},method,path,&headers,body,None)?;
                json::write_new(output.join(format!("{label}-{id}.json")),&json!({"response":reply.evidence(),"method":method,"path":path,"request_headers":headers,"request_base64":STANDARD.encode(body),"files_before_base64":before.iter().map(|bytes|STANDARD.encode(bytes)).collect::<Vec<_>>(),"files_after_base64":api::files(&api)?.iter().map(|bytes|STANDARD.encode(bytes)).collect::<Vec<_>>()}))?;
                require(api::files(&api)?==before,format!("{label}/{id} rejected request changed durable files"))?;
                let diagnostic=sharing::diagnostic(&reply.value()?,mode,kind,*status,detail,*exact,false).map_err(|error|format!("{label}/{id}: {error}"))?;
                observed.push(json!({"case_id":id,"status":status,"diagnostic":diagnostic,"state":api.durable()?,"root":api.stored("root.json")?}));
            }paired.push(observed);
        }json::check(&json!(paired[0]),&json!(paired[1]),false)?;
    }Ok(())
}

pub fn rejected_startups(output:&Path)->Result{
    let Backend::Rust(binary)=service::target()? else{return Err("Corrupt native startup acceptance requires the native executable".into())};
    let old_data=json::stringify(&crate::edges::seed_document()).into_bytes();let old_root=b"{\"opaque\":true}";
    let mut cases=vec![("corrupt-state",b"invalid backup \xff".to_vec())];
    for(label,document)in[
        ("invalid-legacy-record",json!({"settings":{},"subs":[null]})),("invalid-legacy-processes",json!({"settings":{},"subs":[{"process":{}}]})),
        ("invalid-legacy-process",json!({"settings":{},"subs":[{"process":[null]}]})),("invalid-legacy-process-type",json!({"settings":{},"subs":[{"process":[{"type":true}]}]})),
        ("missing-legacy-operator-args",json!({"settings":{},"subs":[{"process":[{"type":"Set Property Operator"}]}]})),("invalid-other-family",json!({"schemaVersion":"2.0","settings":{},"artifacts":[null]})),
        ("invalid-buffer-byte",json!({"schemaVersion":"2.0","settings":{},"subs":{"type":"Buffer","data":[256]}})),("invalid-buffer-shape",json!({"schemaVersion":"2.0","settings":{},"collections":{"type":"Buffer","data":[0],"extra":true}}))
    ]{cases.push((label,json::stringify(&document).into_bytes()));}
    for(label,original)in cases{
        let state=output.join(label);std::fs::create_dir(&state)?;std::fs::write(state.join("sub-store.json"),&original)?;std::fs::write(state.join("root.json"),old_root)?;
        let result=process::run(Command::new(&binary).current_dir(crate::root()).env_clear().envs(server::environment(&state)),output,label,Duration::from_secs(10))?;
        require(!result.timed_out&&!result.status.success()&&std::fs::read(state.join("sub-store.json"))?==original&&std::fs::read(state.join("root.json"))?==old_root,"Rejected startup reset files or failed to exit")?;
    }
    let label="corrupt-restore-startup";let state=output.join(label);std::fs::create_dir(&state)?;std::fs::write(state.join("sub-store.json"),&old_data)?;std::fs::write(state.join("root.json"),old_root)?;
    let pending=state.join(".sub-store-restore.pending.json");let invalid=json::stringify(&json!({"data":STANDARD.encode(&old_data),"root":"invalid!"})).into_bytes();std::fs::write(&pending,&invalid)?;
    let result=process::run(Command::new(&binary).current_dir(crate::root()).env_clear().envs(server::environment(&state)),output,label,Duration::from_secs(10))?;
    require(!result.timed_out&&!result.status.success()&&std::fs::read(&pending)?==invalid&&std::fs::read(state.join("sub-store.json"))?==old_data&&std::fs::read(state.join("root.json"))?==old_root,"Corrupt restore journal reset persisted data or failed to exit")
}
