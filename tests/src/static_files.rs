use crate::{Result, api::{self,Api}, buffers, entities, fixture::{quote,unquote}, json, media, persistence, raw_http, reference, require, server::{self,Backend}, service};
use base64::{Engine,engine::general_purpose::STANDARD};
use serde_json::{Value,json};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime};

pub fn prepare(frontend: &Path) -> Result {
    let files = [
        (
            "index.html",
            b"<main>unchanged frontend entry</main>".as_slice(),
        ),
        (
            "chunks/main-84da2e86.js",
            b"console.log(\"original chunk\")".as_slice(),
        ),
        (
            "chunks/multi-deadbeef.locale-extra.js",
            b"hashed suffix".as_slice(),
        ),
        (
            "chunks/multi-deadbeef.-extra.js",
            b"hashed dash suffix".as_slice(),
        ),
        ("css/main.css", b"body{}".as_slice()),
        ("sw.js", b"worker".as_slice()),
        ("empty.txt", b"".as_slice()),
        (
            "release-deadbeef.dir/index.html",
            b"nested entry".as_slice(),
        ),
        ("text 中文.txt", "Unicode中文🙂\r\n".as_bytes()),
        (".env", b"must never expose dotfiles".as_slice()),
    ];
    let write = |name: &str, bytes: &[u8]| -> Result {
        let path = frontend.join(name);
        std::fs::create_dir_all(path.parent().ok_or("Missing fixture parent")?)?;
        std::fs::write(&path, bytes)?;
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1609459200);
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?
            .set_times(
                std::fs::FileTimes::new()
                    .set_accessed(time)
                    .set_modified(time),
            )?;
        Ok(())
    };
    let gzip = |bytes: &[u8]| -> Result<Vec<u8>> {
        let mut encoder = flate2::GzBuilder::new()
            .mtime(0)
            .write(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes)?;
        Ok(encoder.finish()?)
    };
    for (name, bytes) in files {
        write(name, bytes)?;
        if ["index.html", "chunks/main-84da2e86.js", "css/main.css"].contains(&name) {
            write(&format!("{name}.gz"), &gzip(bytes)?)?;
        }
    }
    write(
        "chunks/only-deadbeef.mjs.gz",
        &gzip(b"compressed-only module")?,
    )?;
    std::fs::write(
        frontend
            .parent()
            .ok_or("Missing fixture state")?
            .join("secret.txt"),
        b"must never expose parent files",
    )?;
    Ok(())
}

const CONTRACT_HEADERS:[&str;17]=["content-type","content-length","content-encoding","cache-control","etag","last-modified","vary","accept-ranges","content-disposition","x-powered-by","access-control-allow-origin","access-control-allow-methods","access-control-allow-headers","content-security-policy","x-content-type-options","content-range","allow"];
struct Resources<'a>{api:&'a Api,base:&'a str,output:&'a Path,label:&'a str,standalone:bool,observed:Vec<Value>}
impl Resources<'_>{
    fn send(&mut self,path:&str,spec:Value,expected:Option<&[u8]>)->Result<raw_http::Reply>{
        let method=spec["method"].as_str().unwrap_or("GET");let status=spec["status"].as_u64().unwrap_or(200) as u16;let is_api=spec["api"]==true;
        let mut headers=BTreeMap::new();if self.standalone{headers.insert("Origin".into(),"https://allowed.invalid".into());}
        if let Some(fields)=spec["headers"].as_object(){for(key,value)in fields{headers.insert(key.clone(),value.as_str().ok_or("Invalid static fixture header")?.into());}}
        let body=spec.get("body").map(json::stringify).unwrap_or_default().into_bytes();if spec.get("body").is_some(){headers.insert("Content-Type".into(),"application/json".into());}
        let lower=service::clock(false);let reply=raw_http::request(self.base,method,path,&headers,&body,None)?;let upper=service::clock(true);
        json::write_new(self.output.join(format!("{}-{:04}.json",self.label,self.observed.len())),&json!({"path":path,"method":method,"request_headers":headers,"request_body_base64":STANDARD.encode(body),"response":reply.evidence(),"lower_ms":lower,"upper_ms":upper}))?;
        require(reply.status==status,format!("{} {method} {path}: expected {status}, actual {}",self.label,reply.evidence()))?;
        if let Some(expected)=expected{require(reply.body==expected,format!("{} {method} {path}: static response bytes changed",self.label))?;}
        if self.standalone{
            require(reply.headers.get("x-powered-by").map(String::as_str)==Some(if is_api{"Sub-Store"}else{"Express"}),"Standalone response used the wrong routing owner")?;
            if !is_api{require(!reply.headers.contains_key("access-control-allow-origin"),"Static frontend inherited backend CORS")?;}
        }
        if is_api&&reply.headers.contains_key("content-disposition"){
            let resource=unquote(path.split('?').next().unwrap().rsplit('/').next().ok_or("Missing exported name")?)?;
            persistence::filename(&reply.value()?,&format!("sub-store_subscription_{resource}"),lower,upper,&jiff::tz::TimeZone::system())?;
        }
        let compared=if is_api{["content-type","x-powered-by","vary","access-control-allow-origin","access-control-allow-methods","access-control-allow-headers"].as_slice()}else if self.standalone{CONTRACT_HEADERS.as_slice()}else{["content-type","content-length","content-encoding","cache-control","etag","last-modified","vary","accept-ranges","content-range","content-security-policy","x-content-type-options"].as_slice()};
        let mut record=json!({"method":method,"path":path,"status":status,"headers":reply.headers.iter().filter(|(key,_)|compared.contains(&key.as_str())).collect::<BTreeMap<_,_>>(),"body_base64":STANDARD.encode(&reply.body)});
        if self.standalone{record["state"]=self.api.durable()?;if is_api&&reply.body.starts_with(b"{"){record["body"]=json::parse(std::str::from_utf8(&reply.body)?)?;record.as_object_mut().unwrap().shift_remove("body_base64");}}
        self.observed.push(record);Ok(reply)
    }
    fn bytes(&mut self,path:&str,headers:Value,method:&str,status:u16,expected:&[u8])->Result<raw_http::Reply>{self.send(path,json!({"headers":headers,"method":method,"status":status}),Some(expected))}
    fn range(&mut self,label:&str,headers:Value,status:u16,expected:&[u8],method:&str,content_range:Option<String>)->Result{
        let reply=self.bytes(&format!("/chunks/main-84da2e86.js?conditional={label}"),headers,method,status,if method=="HEAD"{b""}else{expected})?;
        require(reply.headers.get("content-range")==content_range.as_ref(),format!("{label}: incorrect Content-Range"))?;
        if status!=304{let length=expected.len().to_string();require(reply.headers.get("content-length")==Some(&length),format!("{label}: incorrect representation length"))?;}
        if status>=400{require(reply.headers.get("content-type").map(String::as_str)==Some("text/html; charset=utf-8")&&reply.headers.get("content-security-policy").map(String::as_str)==Some("default-src 'none'")&&reply.headers.get("x-content-type-options").map(String::as_str)==Some("nosniff")&&!reply.headers.contains_key("content-encoding"),"Static precondition/range error headers changed")?;}Ok(())
    }
    fn conditions(&mut self,frontend:&Path)->Result{
        let path="/chunks/main-84da2e86.js";let plain=std::fs::read(frontend.join(&path[1..]))?;let compressed=std::fs::read(frontend.join(format!("{}.gz",&path[1..])))?;
        let normal=self.bytes(&format!("{path}?conditional=plain"),json!({}),"GET",200,&plain)?;let encoded=self.bytes(&format!("{path}?conditional=gzip"),json!({"Accept-Encoding":"gzip"}),"GET",200,&compressed)?;
        let error=|status|format!("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error</title>\n</head>\n<body>\n<pre>{}</pre>\n</body>\n</html>\n",if status==412{"Precondition Failed"}else{"Range Not Satisfiable"}).into_bytes();let precondition=error(412);let unsatisfied=error(416);let length=plain.len();
        for(label,value,start,end)in[("first","bytes=0-3".into(),0,3),("open","bytes=4-".into(),4,length-1),("suffix","bytes=-4".into(),length-4,length-1),("clamped","bytes=2-99999999999999999999999".into(),2,length-1),("whole-suffix",format!("bytes=-{length}"),0,length-1),("adjacent","bytes=3-5,0-2".into(),0,5),("overlap","bytes=2-5,0-3".into(),0,5),("duplicate","bytes=0-3,0-3".into(),0,3),("one-valid","bytes=99999-,1-4".into(),1,4),("numeric-prefix","bytes=1garbage-4suffix".into(),1,4),("suffix-coercion","bytes=invalid-4".into(),length-4,length-1),("spaces","bytes= 1 - 4 ".into(),1,4)]{self.range(label,json!({"Range":value}),206,&plain[start..=end],"GET",Some(format!("bytes {start}-{end}/{length}")))?;}
        for(label,value)in[("disjoint","bytes=0-1,4-5"),("other-unit","items=0-3"),("uppercase","Bytes=0-3"),("no-equals","bytes"),("space-before-equals","bytes =0-3")]{self.range(label,json!({"Range":value}),200,&plain,"GET",None)?;}
        for(label,value)in[("past-end",format!("bytes={length}-")),("reversed","bytes=5-2".into()),("zero-suffix","bytes=-0".into()),("large-suffix",format!("bytes=-{}",length+1)),("empty-range","bytes=".into()),("invalid-range","bytes=garbage".into())]{self.range(label,json!({"Range":value}),416,&unsatisfied,"GET",Some(format!("bytes */{length}")))?;}
        self.range("head-range",json!({"Range":"bytes=0-3"}),206,&plain[..4],"HEAD",Some(format!("bytes 0-3/{length}")))?;self.range("head-unsatisfied",json!({"Range":"bytes=99999-"}),416,&unsatisfied,"HEAD",Some(format!("bytes */{length}")))?;
        let tag=&normal.headers["etag"];let modified=&normal.headers["last-modified"];
        for(label,value,fresh)in[("weak",tag.clone(),true),("strong",tag[2..].into(),false),("missing","\"miss\"".into(),false),("date",modified.clone(),true),("future","Sat, 02 Jan 2021 00:00:00 GMT".into(),true),("old","Thu, 31 Dec 2020 23:59:59 GMT".into(),false),("invalid","invalid".into(),false)]{self.range(&format!("if-range-{label}"),json!({"Range":"bytes=0-3","If-Range":value}),if fresh{206}else{200},if fresh{&plain[..4]}else{&plain},"GET",fresh.then(||format!("bytes 0-3/{length}")))?;}
        self.range("stale-unsatisfied",json!({"Range":"bytes=99999-","If-Range":"\"miss\""}),200,&plain,"GET",None)?;
        for(label,value)in[("weak",tag.clone()),("strong",tag[2..].into()),("list",format!("\"miss\", {tag}")),("wildcard","*".into())]{self.range(&format!("if-match-{label}"),json!({"If-Match":value}),200,&plain,"GET",None)?;}
        self.range("if-match-miss",json!({"If-Match":"\"miss\""}),412,&precondition,"GET",None)?;self.range("head-precondition",json!({"If-Match":"\"miss\""}),412,&precondition,"HEAD",None)?;
        for(label,value)in[("equal",modified.as_str()),("future","Sat, 02 Jan 2021 00:00:00 GMT"),("invalid","invalid")]{self.range(&format!("unmodified-{label}"),json!({"If-Unmodified-Since":value}),200,&plain,"GET",None)?;}
        let old="Thu, 31 Dec 2020 23:59:59 GMT";self.range("unmodified-old",json!({"If-Unmodified-Since":old}),412,&precondition,"GET",None)?;self.range("match-over-date",json!({"If-Match":tag,"If-Unmodified-Since":old}),200,&plain,"GET",None)?;
        self.range("precondition-before-fresh",json!({"If-Match":"\"miss\"","If-None-Match":tag}),412,&precondition,"GET",None)?;self.range("precondition-before-range",json!({"If-Match":"\"miss\"","Range":"bytes=0-3"}),412,&precondition,"GET",None)?;self.range("fresh-before-range",json!({"If-None-Match":tag,"Range":"bytes=99999-"}),304,b"","GET",None)?;self.range("matched-range",json!({"If-Match":tag,"Range":"bytes=0-3"}),206,&plain[..4],"GET",Some(format!("bytes 0-3/{length}")))?;
        let gzip=json!({"Accept-Encoding":"gzip","Range":"bytes=0-3"});let encoded_length=compressed.len();self.range("gzip-range",gzip.clone(),206,&compressed[..4],"GET",Some(format!("bytes 0-3/{encoded_length}")))?;self.range("gzip-range-head",gzip,206,&compressed[..4],"HEAD",Some(format!("bytes 0-3/{encoded_length}")))?;
        self.range("gzip-if-range",json!({"Accept-Encoding":"gzip","Range":"bytes=0-3","If-Range":encoded.headers["etag"]}),206,&compressed[..4],"GET",Some(format!("bytes 0-3/{encoded_length}")))?;self.range("gzip-stale-if-range",json!({"Accept-Encoding":"gzip","Range":"bytes=0-3","If-Range":tag}),200,&compressed,"GET",None)?;self.range("gzip-precondition",json!({"Accept-Encoding":"gzip","Range":"bytes=0-3","If-Match":tag}),412,&precondition,"GET",None)?;self.range("gzip-unsatisfied",json!({"Accept-Encoding":"gzip","Range":"bytes=99999-"}),416,&unsatisfied,"GET",Some(format!("bytes */{encoded_length}")))?;
        self.bytes("/empty.txt?conditional=empty",json!({}),"GET",200,b"")?;let empty=self.bytes("/empty.txt?conditional=range",json!({"Range":"bytes=0-"}),"GET",416,&unsatisfied)?;require(empty.headers.get("content-range").map(String::as_str)==Some("bytes */0"),"Empty representation range boundary changed")
    }
    fn merged(&mut self)->Result{
        let frontend=self.api.state.join("frontend");let before=api::files(self.api)?;let path="/chunks/main-84da2e86.js";let plain=std::fs::read(frontend.join(&path[1..]))?;let compressed=std::fs::read(frontend.join(format!("{}.gz",&path[1..])))?;
        let normal=self.bytes(&format!("{path}?cache=1"),json!({}),"GET",200,&plain)?;let encoded=self.bytes(path,json!({"Accept-Encoding":"gzip"}),"GET",200,&compressed)?;
        let length=plain.len().to_string();let encoded_length=compressed.len().to_string();require(normal.headers["content-type"]=="application/javascript; charset=utf-8"&&normal.headers["content-length"]==length&&encoded.headers["content-length"]==encoded_length&&normal.headers["cache-control"]=="public, max-age=31536000, immutable"&&encoded.headers["content-encoding"]=="gzip"&&!normal.headers.contains_key("content-encoding")&&normal.headers["last-modified"]=="Fri, 01 Jan 2021 00:00:00 GMT"&&json::full_match(r#"W/"[0-9a-f]+-[0-9a-f]+""#,&encoded.headers["etag"])?,"Static representation cache/type/length/mtime/ETag contract changed")?;
        self.bytes(path,json!({"Accept-Encoding":"gzip"}),"HEAD",200,b"")?;
        for(accept,gzip)in[("gzip;q=0",false),("gzip;q=0.5",true),("gzip;q=0.5, identity;q=1",false),("identity;q=0.5, gzip;q=1",true),("*;q=0.8",true),("*;q=0",false),("gzip;q=0, *;q=1",false),("GZIP; q=0.8",true),("br, gzip;q=.8",true),("identity, gzip",false),("gzip, identity",true),("gzip;q=0.4, gzip;q=0.8, identity;q=0.6",true),("gzip;q=invalid",false),("gzip;q=0.8suffix",true)]{self.bytes(path,json!({"Accept-Encoding":accept}),"GET",200,if gzip{&compressed}else{&plain})?;}
        let tag=&encoded.headers["etag"];for matched in[tag.clone(),tag[2..].into(),format!("\"miss\", {tag}"),"*".into()]{let reply=self.bytes(path,json!({"Accept-Encoding":"gzip","If-None-Match":matched}),"GET",304,b"")?;require(["content-type","content-length","content-encoding"].iter().all(|key|!reply.headers.contains_key(*key)),"Static 304 retained representation headers")?;}
        self.bytes(path,json!({"Accept-Encoding":"gzip","If-None-Match":tag}),"HEAD",304,b"")?;self.bytes(path,json!({"If-Modified-Since":normal.headers["last-modified"]}),"GET",304,b"")?;
        for headers in[json!({"If-Modified-Since":"invalid"}),json!({"If-Modified-Since":"Thu, 31 Dec 2020 23:59:59 GMT"}),json!({"If-None-Match":normal.headers["etag"],"Cache-Control":"max-age=0, no-cache"}),json!({"If-None-Match":normal.headers["etag"],"If-Modified-Since":"invalid"})]{self.bytes(path,headers,"GET",200,&plain)?;}
        self.bytes(path,json!({"Accept-Encoding":"gzip","If-None-Match":normal.headers["etag"]}),"GET",200,&compressed)?;self.bytes("/sw.js",json!({"Accept-Encoding":"gzip"}),"GET",200,b"worker")?;
        for(path,file)in[("/css/main.css?cache=1","css/main.css.gz"),("/chunks/only-deadbeef.mjs","chunks/only-deadbeef.mjs.gz")]{self.bytes(path,json!({"Accept-Encoding":"gzip"}),"GET",200,&std::fs::read(frontend.join(file))?)?;}
        self.bytes(&format!("{path}.gz"),json!({}),"GET",200,&compressed)?;self.bytes("/text%20%E4%B8%AD%E6%96%87.txt",json!({}),"GET",200,&std::fs::read(frontend.join("text 中文.txt"))?)?;self.bytes("/release-deadbeef.dir/index.html",json!({}),"GET",200,b"nested entry")?;
        for(suffix,body)in[("locale-extra",b"hashed suffix".as_slice()),("-extra",b"hashed dash suffix".as_slice())]{let reply=self.bytes(&format!("/chunks/multi-deadbeef.{suffix}.js"),json!({}),"GET",200,body)?;require(reply.headers["cache-control"]=="public, max-age=31536000, immutable","Hashed multi-suffix asset cache changed")?;}
        let entry=std::fs::read(frontend.join("index.html"))?;for path in["/","/settings","/missing.js","/.env","/%2e%2e%2fsecret.txt"]{let reply=self.bytes(path,json!({}),"GET",200,&entry)?;require(reply.headers["cache-control"]=="public, max-age=0","SPA fallback cache changed")?;}
        self.bytes("/settings",json!({"Accept-Encoding":"gzip"}),"GET",200,&std::fs::read(frontend.join("index.html.gz"))?)?;for path in["/api/not-a-resource","/download/not-a-route/missing/extra/segment","/share/not-a-resource"]{self.bytes(path,json!({}),"GET",404,b"")?;}
        self.conditions(&frontend)?;
        let mut encoder=flate2::GzBuilder::new().mtime(0).write(Vec::new(),flate2::Compression::default());encoder.write_all(b"console.log(\"replacement chunk is visible\")")?;let replacement=encoder.finish()?;let path_on_disk=frontend.join(format!("{}.gz",&path[1..]));std::fs::write(&path_on_disk,&replacement)?;let file=std::fs::OpenOptions::new().write(true).open(&path_on_disk)?;let modified=SystemTime::UNIX_EPOCH+Duration::from_secs(1609459202);file.set_times(std::fs::FileTimes::new().set_accessed(modified).set_modified(modified))?;
        let refreshed=self.bytes(path,json!({"Accept-Encoding":"gzip","If-None-Match":tag}),"GET",200,&replacement)?;require(refreshed.headers.get("etag")!=encoded.headers.get("etag")&&refreshed.headers.get("last-modified")!=encoded.headers.get("last-modified"),"Changed on-disk asset retained old validators")?;require(api::files(self.api)?==before,"Static request changed durable state")?;
        self.observed.extend(entities::run(self.api,&self.api.listener.url,None,self.output,&format!("{}-api-http",self.label))?);Ok(())
    }
    fn standalone(&mut self,mount:&str)->Result{
        let frontend=self.api.state.join("frontend");let before=api::files(self.api)?;let path="/chunks/main-84da2e86.js";let entry=std::fs::read(frontend.join("index.html"))?;
        let normal=self.bytes(&format!("{path}?cache=1"),json!({}),"GET",200,&std::fs::read(frontend.join(&path[1..]))?)?;self.bytes(path,json!({"Accept-Encoding":"gzip"}),"GET",200,&std::fs::read(frontend.join(format!("{}.gz",&path[1..])))?)?;self.bytes(path,json!({}),"HEAD",200,b"")?;self.bytes(path,json!({"If-None-Match":normal.headers["etag"]}),"GET",304,b"")?;
        self.bytes("/",json!({}),"GET",200,&entry)?;for path in["/settings","/missing.js","/.env","/%2e%2e%2fsecret.txt"]{self.bytes(path,json!({"Accept":"text/html"}),"GET",200,&entry)?;}
        self.bytes("/settings",json!({"Accept":"*/*","Accept-Encoding":"gzip"}),"GET",200,&std::fs::read(frontend.join("index.html.gz"))?)?;self.bytes("/settings",json!({"Accept":"text/html"}),"HEAD",200,b"")?;
        for headers in[json!({}),json!({"Accept":"application/json"}),json!({"Accept":"application/json, text/html"})]{self.send("/settings",json!({"headers":headers,"status":404}),None)?;}
        self.send("/settings?query=1",json!({"headers":{"Accept":"text/html"},"method":"POST","status":404}),None)?;self.conditions(&frontend)?;require(api::files(self.api)?==before,"Static frontend changed state")?;
        if !mount.is_empty(){
            let prefix=format!("{}/api",mount.trim_end_matches('/'));
            self.send(&format!("{prefix}/settings"),json!({"api":true}),None)?;self.send(&format!("{prefix}/settings?keep=%E4%B8%AD&keep=%2B"),json!({"api":true,"method":"PATCH","body":{"standalone":"转交🙂","zero":0}}),None)?;require(self.api.data(&json!({"path":"/api/settings"}))?["standalone"]=="转交🙂","Proxy wrote a different store")?;
            self.send(&format!("{prefix}/subs"),json!({"api":true,"method":"POST","status":201,"body":{"name":"独立 %?","url":"https://example.invalid/list"}}),None)?;
            let encoded=quote("独立 %?");let exported=self.send(&format!("{prefix}/sub/{encoded}?raw=1&opaque=%26%2B"),json!({"api":true}),None)?;require(exported.headers.contains_key("content-disposition"),"Proxy lost export headers")?;
            self.send(&format!("{prefix}/sub/{encoded}"),json!({"api":true,"method":"PATCH","body":{"name":"renamed","custom":false}}),None)?;self.send(&format!("{prefix}/sub/renamed"),json!({"api":true,"method":"DELETE"}),None)?;self.send(&format!("{prefix}/settings"),json!({"api":true,"method":"OPTIONS"}),Some(b""))?;
            self.send(&format!("{prefix}/settings"),json!({"api":true,"headers":{"Origin":"https://denied.invalid"},"method":"PATCH","status":403,"body":{"mustNotWrite":true}}),Some(b"CORS origin not allowed"))?;require(self.api.durable()?["settings"].get("mustNotWrite").is_none(),"Denied proxy request changed settings")?;
            self.send(&format!("{}/download/not-a-route/missing/extra/segment?raw=1",mount.trim_end_matches('/')),json!({"api":true,"status":404}),Some(b""))?;self.send(&format!("{}/settings",prefix.to_uppercase()),json!({"api":true}),None)?;self.bytes(&format!("{prefix}extra/settings"),json!({"Accept":"text/html"}),"GET",200,&entry)?;
            let base=format!("{}{}",self.base,mount.trim_end_matches('/'));self.observed.extend(entities::run(self.api,&base,Some("https://allowed.invalid"),self.output,&format!("{}-api-http",self.label))?);
            if mount=="/backend"{self.observed.extend(media::run(self.api,&base,self.output,&format!("{}-body-http",self.label),Some("https://allowed.invalid"))?);for phase in["live","restore"]{self.observed.extend(buffers::phase(self.api,&base,self.output,&format!("{}-buffers-{phase}",self.label),phase,Some("https://allowed.invalid"))?);}}
        }else{self.send("/api/settings",json!({"status":404}),None)?;self.bytes("/api/settings",json!({"Accept":"text/html"}),"GET",200,&entry)?;}
        Ok(())
    }
}
pub fn run(output:&Path)->Result{
    reference::verify(output)?;let backup=json::read(crate::compat().join("fixtures/frontend-backup.json"))?;let root=json!({"opaque":{"text":"cache🙂","zero":0,"flag":false},"sub-store-cached-resource":"{}"});
    for(mode,mount,prefix,merged)in[("merged-prefix","/backend","true",true),("merged-root","/","true",true),("prefixed","/backend","true",false),("unprefixed","/gateway","",false),("root","/","true",false),("static-only","","",false)]{
        let mut paired=Vec::new();
        for(kind,backend)in[("node",Backend::Node(reference::node())),("rust",service::target()?)]{
            let label=format!("{kind}-static-{mode}");let(state,mut env)=service::seed(output,&label,&backup,&root)?;prepare(&state.join("frontend"))?;
            for(key,value)in[("NODE_ENV","production"),("SUB_STORE_BACKEND_PREFIX",prefix),("SUB_STORE_BACKEND_MERGE",if merged{"true"}else{""}),("SUB_STORE_FRONTEND_BACKEND_PATH",mount),("SUB_STORE_FRONTEND_HOST","127.0.0.1"),("SUB_STORE_BODY_JSON_LIMIT","32kb")]{server::set(&mut env,key,value);}server::set(&mut env,"SUB_STORE_FRONTEND_PATH",state.join("frontend"));server::set(&mut env,"SUB_STORE_FRONTEND_PORT",server::free_port()?.to_string());if !merged{server::set(&mut env,"SUB_STORE_CORS_ALLOWED_ORIGINS","https://allowed.invalid");}
            let api=Api::start(&backend,&env,&state,output,&label)?;if !merged{api::wait(||Ok(api.http.request(&api.listener.frontend,&json!({"path":"/"}))?["status"]==200))?;}
            let base=if merged{&api.listener.origin}else{&api.listener.frontend};let mut test=Resources{api:&api,base,output,label:&label,standalone:!merged,observed:Vec::new()};if merged{test.merged()?;}else{test.standalone(mount)?;}
            if mode=="prefixed"{let before=api::files(&api)?;let reply=raw_http::request(&api.listener.frontend,"GET","/share/sub/renamed?token=invalid",&BTreeMap::new(),b"",None)?;require(reply.status==404&&reply.body.is_empty()&&api::files(&api)?==before,"Invalid standalone share consumed state")?;}
            json::write_new(output.join(format!("{label}.json")),&json!(test.observed))?;paired.push(test.observed);
        }json::check(&json!(paired[0]),&json!(paired[1]),false)?;
    }Ok(())
}
