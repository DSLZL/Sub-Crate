use crate::{Result,api::{self,Api},fixture::{quote,unquote},json,media,persistence,raw_http,require,service};
use base64::{Engine,engine::general_purpose::STANDARD};
use serde_json::{Value,json};
use std::{collections::BTreeMap,path::Path,time::Duration};

const HEADERS:[&str;10]=["content-type","content-length","etag","last-modified","content-encoding","cache-control","vary","content-security-policy","x-content-type-options","allow"];
struct Entities<'a>{api:&'a Api,base:&'a str,origin:Option<&'a str>,output:&'a Path,label:&'a str,observed:Vec<Value>}
impl Entities<'_>{
    fn send(&mut self,path:&str,spec:Value)->Result<raw_http::Reply>{
        let method=spec["method"].as_str().unwrap_or("GET");let status=spec["status"].as_u64().unwrap_or(200) as u16;let tagged=spec["tagged"]!=false;
        let mut headers=BTreeMap::new();if let Some(origin)=self.origin{headers.insert("Origin".into(),origin.into());}
        if let Some(fields)=spec["headers"].as_object(){for(key,value)in fields{if value.is_null(){headers.remove(key);}else{headers.insert(key.clone(),value.as_str().ok_or("Invalid entity fixture header")?.into());}}}
        let body=if let Some(raw)=spec["raw"].as_str(){raw.as_bytes().to_vec()}else if let Some(body)=spec.get("body"){json::stringify(body).into_bytes()}else{Vec::new()};
        if spec.get("raw").is_some()||spec.get("body").is_some(){headers.entry("Content-Type".into()).or_insert_with(||"application/json".into());}
        let lower=service::clock(false);let reply=raw_http::request(self.base,method,path,&headers,&body,None)?;let upper=service::clock(true);
        json::write_new(self.output.join(format!("{}-{:04}.json",self.label,self.observed.len())),&json!({"path":path,"method":method,"request_headers":headers,"request_base64":STANDARD.encode(body),"response":reply.evidence(),"lower_ms":lower,"upper_ms":upper,"state":self.api.durable()?}))?;
        require(reply.status==status,format!("{} {method} {path}: expected {status}, actual {}",self.label,reply.evidence()))?;
        if let Some(expected)=spec["expected"].as_str(){require(reply.body==expected.as_bytes(),"Entity response body changed")?;}
        if tagged{
            require(reply.headers.contains_key("etag"),"API entity ETag missing")?;
            if method!="HEAD"&&status!=304{let length=reply.body.len().to_string();require(reply.headers.get("etag")==Some(&media::etag(&reply.body))&&reply.headers.get("content-length")==Some(&length),"API ETag/length differs from actual bytes")?;}
        }else{require(!reply.headers.contains_key("etag"),"Finalhandler/end response gained an entity tag")?;}
        if status==304{require(reply.body.is_empty()&&["content-type","content-length","transfer-encoding"].iter().all(|key|!reply.headers.contains_key(*key)),"API 304 retained representation body/headers")?;}
        if let Some(disposition)=reply.headers.get("content-disposition"){
            let captures=regex::Regex::new(r#"^attachment; filename="(.*)_\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}\.json"$"#)?.captures(disposition).ok_or("Invalid entity export filename")?;
            persistence::filename(&reply.value()?,&unquote(&captures[1])?,lower,upper,&jiff::tz::TimeZone::system())?;
        }
        if let Some(origin)=headers.get("Origin").filter(|origin|!origin.is_empty()){if status!=403{require(reply.headers.get("access-control-allow-origin")==Some(origin),"API CORS origin changed")?;}}
        self.observed.push(json!({"method":method,"path":path,"status":status,"headers":reply.headers.iter().filter(|(key,_)|HEADERS.contains(&key.as_str())).collect::<BTreeMap<_,_>>(),"body_base64":STANDARD.encode(&reply.body),"state":self.api.durable()?}));
        Ok(reply)
    }
    fn same(&mut self,path:&str,headers:Value,body:&[u8],status:u16,method:&str)->Result<raw_http::Reply>{
        let reply=self.send(path,json!({"headers":headers,"status":status,"method":method}))?;require(reply.body==body,"Entity revalidation changed representation bytes")?;Ok(reply)
    }
    fn value(reply:&raw_http::Reply)->Result<Value>{json::parse(std::str::from_utf8(&reply.body)?)}
    fn run(&mut self)->Result{
        let payload=json!({"name":"entity-保留🙂","text":"Unicode🙂\r\n","zero":0,"nil":null});
        self.send("/api/collections",json!({"method":"POST","status":201,"body":payload}))?;
        let item=format!("/api/collection/{}",quote("entity-保留🙂"));self.send("/api/settings",json!({"method":"PATCH","body":{"entityValidator":"before"}}))?;
        let before=api::files(self.api)?;
        for _ in 0..3{let time=service::clock(true);let offset=if cfg!(windows)&&self.label.starts_with("node-"){50}else{0};std::thread::sleep(Duration::from_millis((1000-time.rem_euclid(1000)+offset)as u64));self.send("/api/storage",json!({}))?;}
        for path in["/api/settings","/api/subs","/api/collections",&item,&format!("{item}?raw=1"),"/api/storage"]{
            let normal=self.send(path,json!({}))?;let head=self.send(path,json!({"method":"HEAD","expected":""}))?;
            require(HEADERS.iter().all(|key|head.headers.get(*key)==normal.headers.get(*key)),"HEAD entity headers changed")?;
            let tag=normal.headers.get("etag").ok_or("Missing normal tag")?;
            for matched in[tag.clone(),tag[2..].into(),format!("\"miss\", {tag}"),"*".into()]{let cached=self.send(path,json!({"headers":{"If-None-Match":matched},"status":304,"expected":""}))?;require(cached.headers.get("etag")==Some(tag),"304 ETag changed")?;}
            self.send(path,json!({"headers":{"If-None-Match":tag},"method":"HEAD","status":304,"expected":""}))?;
            for headers in[json!({"If-None-Match":"\"miss\""}),json!({"If-None-Match":tag,"Cache-Control":"max-age=0, no-cache"}),json!({"If-None-Match":tag,"If-Modified-Since":"Fri, 01 Jan 2100 00:00:00 GMT"}),json!({"If-None-Match":tag,"If-Modified-Since":"invalid"}),json!({"If-Modified-Since":"Fri, 01 Jan 2100 00:00:00 GMT"}),json!({"If-None-Match":format!("\"miss\",\t{tag}")}),json!({"If-Match":"\"miss\"","If-Unmodified-Since":"Fri, 01 Jan 1999 00:00:00 GMT","Range":"bytes=0-1"})]{let fresh=self.same(path,headers,&normal.body,200,"GET")?;require(fresh.headers.get("etag")==Some(tag)&&!fresh.headers.contains_key("last-modified"),"API fresh tag or Last-Modified changed")?;}
        }require(api::files(self.api)?==before,"API revalidation changed durable files")?;
        let previous=self.send("/api/settings",json!({}))?;let changed=self.send("/api/settings",json!({"method":"PATCH","headers":{"If-None-Match":"*","If-Match":"\"miss\""},"body":{"entityValidator":"after"}}))?;
        require(previous.headers.get("etag")!=changed.headers.get("etag")&&self.api.durable()?["settings"]["entityValidator"]=="after","Mutation incorrectly obeyed read validators")?;
        self.send("/api/settings",json!({"headers":{"If-None-Match":previous.headers["etag"]}}))?;self.send("/api/settings",json!({"headers":{"If-None-Match":changed.headers["etag"]},"status":304,"expected":""}))?;
        self.send("/api/collections",json!({"headers":{"If-None-Match":"*"},"method":"POST","body":payload,"status":500}))?;
        self.send(&format!("{item}?mode=permanent"),json!({"headers":{"If-None-Match":"*"},"method":"DELETE"}))?;
        self.send(&item,json!({"headers":{"If-None-Match":"*"},"status":500}))?;self.send(&item,json!({"method":"HEAD","status":500,"expected":""}))?;
        self.send("/api/sub/missing-entity",json!({"headers":{"If-None-Match":"*"},"status":500,"tagged":false}))?;self.send("/api/sub/missing-entity",json!({"method":"HEAD","status":500,"tagged":false,"expected":""}))?;
        self.send("/api/settings",json!({"method":"PATCH","raw":"{","status":400,"tagged":false}))?;
        self.send("/api/not-a-resource",json!({"headers":{"If-None-Match":"*"},"status":404,"tagged":false,"expected":""}))?;
        self.send("/api/settings",json!({"headers":{"Origin":"https://denied.invalid","If-None-Match":"*"},"status":403,"tagged":false,"expected":"CORS origin not allowed"}))?;
        let allowed=self.origin.map(str::to_owned).unwrap_or_else(||{let url=reqwest::Url::parse(self.base).unwrap();format!("{}://{}:{}",url.scheme(),url.host_str().unwrap(),url.port_or_known_default().unwrap())});
        self.send("/api/settings",json!({"headers":{"Origin":allowed,"If-None-Match":"*"},"method":"OPTIONS","tagged":false,"expected":""}))?;
        for resource in["SETTINGS","SuBs","COLLECTIONS","wholeFiles","FILES","MODULES","TOKENS","ARCHIVES"]{self.send(&format!("/api/{resource}/"),json!({}))?;}
        let normal=self.send("/api/SeTTings/",json!({}))?;let head=self.send("/api/SETTINGS/",json!({"method":"HEAD","expected":""}))?;require(HEADERS.iter().all(|key|head.headers.get(*key)==normal.headers.get(*key)),"Mixed-case HEAD changed entity headers")?;
        self.send("/api/SeTTings/",json!({"headers":{"If-None-Match":normal.headers["etag"]},"status":304,"expected":""}))?;
        for(plural,singular)in[("subs","sub"),("collections","collection")]{
            let names=["RouteCase🙂","routecase🙂","route/part","route%2Fpart",""];let mut items=names.iter().enumerate().map(|(index,name)|json!({"name":name,"marker":index})).collect::<Vec<_>>();let mut created=json!({"name":"route-created","marker":"created"});
            if plural=="collections"{for item in items.iter_mut().chain([&mut created]){item["subscriptions"]=json!([]);}}
            self.send(&format!("/api/{}/",plural.to_uppercase()),json!({"method":"PUT","body":items}))?;self.send(&format!("/api/{}/",plural.to_uppercase()),json!({"method":"POST","body":created,"status":201}))?;
            let before=api::files(self.api)?;
            for item in items.iter().take(items.len()-1).chain([&created]){let reply=self.send(&format!("/api/{}/{}/",singular.to_uppercase(),quote(item["name"].as_str().unwrap())),json!({}))?;json::check(&Self::value(&reply)?["data"],item,false)?;}
            let reply=self.send(&format!("/api/{singular}/route%2fpart"),json!({}))?;require(Self::value(&reply)?["data"]["name"]=="route/part","Parameter decoding changed case-insensitive escape")?;
            for path in[format!("/api/{singular}/"),format!("/api/{singular}/route/part")]{self.send(&path,json!({"status":404,"tagged":false,"expected":""}))?;}
            self.send(&format!("/api/{singular}/route-created//"),if self.origin.is_some(){json!({})}else{json!({"status":404,"tagged":false,"expected":""})})?;require(api::files(self.api)?==before,"Route read changed files")?;
            let target=format!("/api/{}/{}/",singular.to_uppercase(),quote(names[0]));self.send(&target,json!({"method":"PATCH","body":{"marker":"patched"}}))?;
            let reply=self.send(&format!("/api/{singular}/{}",quote(names[1])),json!({}))?;require(Self::value(&reply)?["data"]["marker"]==1,"Route case folding changed a different record")?;
            self.send(&format!("{target}?mode=permanent"),json!({"method":"DELETE"}))?;
            let reply=self.send(&format!("/api/{}/",plural.to_uppercase()),json!({}))?;json::check(&json!(Self::value(&reply)?["data"].as_array().ok_or("Missing routed records")?.iter().map(|item|item["name"].clone()).collect::<Vec<_>>()),&json!(names[1..].iter().copied().chain(["route-created"]).collect::<Vec<_>>()),false)?;
        }
        let before=api::files(self.api)?;
        for path in["/api/%73ettings","/api/utils/%65nv"]{self.send(path,json!({"status":404,"tagged":false,"expected":""}))?;}
        self.send("/api/settings//",if self.origin.is_some(){json!({})}else{json!({"status":404,"tagged":false,"expected":""})})?;
        for escape in["%","%0","%GG","%FF","%E0%A4"]{for method in["GET","PATCH"]{let reply=self.send(&format!("/api/sub/{escape}"),json!({"method":method,"status":400,"tagged":false}))?;require(reply.headers.get("content-type").map(String::as_str)==Some("text/html; charset=utf-8")&&reply.headers.get("content-security-policy").map(String::as_str)==Some("default-src 'none'")&&reply.headers.get("x-content-type-options").map(String::as_str)==Some("nosniff")&&String::from_utf8_lossy(&reply.body).contains("Bad Request"),"Rejected parameter diagnostic changed")?;}}
        require(api::files(self.api)?==before,"Rejected parameter changed files")?;
        for method in["POST","PUT","PATCH","DELETE"]{let reply=self.send("/api/method-&'quote?ignored=method-query",json!({"method":method,"status":404,"tagged":false,"body":{"mustNotWrite":true}}))?;let text=std::str::from_utf8(&reply.body)?;require(reply.headers.get("content-type").map(String::as_str)==Some("text/html; charset=utf-8")&&text.contains(&format!("Cannot {method} "))&&text.contains("/api/method-&amp;&#39;quote</pre>")&&!text.contains("method-query"),"Unknown method HTML escaping/query boundary changed")?;}
        for(path,method)in[("/api/settings","POST"),("/api/settings","PUT"),("/api/settings","DELETE"),("/api/subs","PATCH"),("/api/collections","PATCH"),("/api/storage","PATCH"),("/api/storage","DELETE"),("/api/not-a-route/extra","POST")]{let reply=self.send(path,json!({"method":method,"status":404,"tagged":false,"body":{"mustNotWrite":true}}))?;require(reply.headers.get("content-type").map(String::as_str)==Some("text/html; charset=utf-8")&&String::from_utf8_lossy(&reply.body).contains(&format!("Cannot {method} ")),"Wrong method gained a handler")?;}
        for(path,methods)in[("/api/SeTTings/","GET,PATCH,HEAD"),("/api/storage","GET,POST,HEAD"),("/api/subs","GET,POST,PUT,HEAD"),("/api/collections","GET,POST,PUT,HEAD"),("/api/sub/route-created","GET,PATCH,DELETE,HEAD"),("/api/collection/route-created","GET,PATCH,DELETE,HEAD"),("/api/utils/env","GET,HEAD"),("/api/utils/refresh","GET,HEAD"),("/api/not-a-resource","GET,HEAD"),("/api/sub/route/part","GET,HEAD")]{let reply=self.send(path,json!({"headers":{"Origin":null},"method":"OPTIONS","expected":methods}))?;require(reply.headers.get("allow").map(String::as_str)==Some(methods)&&reply.headers.get("content-type").map(String::as_str)==Some("text/plain; charset=utf-8"),"OPTIONS allowed-method inventory changed")?;}
        let reply=self.send("/api/settings",json!({"headers":{"Origin":""},"method":"OPTIONS","expected":"GET,PATCH,HEAD"}))?;require(reply.headers.get("allow").map(String::as_str)==Some("GET,PATCH,HEAD"),"Empty-origin OPTIONS methods changed")?;
        for(origin,status,expected)in[(Value::Null,400,None),(json!(""),400,None),(json!(allowed),200,Some("")),(json!("https://denied.invalid"),403,Some("CORS origin not allowed"))]{let mut spec=json!({"headers":{"Origin":origin},"method":"OPTIONS","raw":"{","status":status,"tagged":false});if let Some(expected)=expected{spec["expected"]=json!(expected);}let reply=self.send("/api/settings",spec)?;require(!reply.headers.contains_key("allow"),"CORS/parser OPTIONS unexpectedly reached method fallback")?;if status==400{require(reply.headers.get("content-type").map(String::as_str)==Some("text/html; charset=utf-8")&&String::from_utf8_lossy(&reply.body).contains("Bad Request"),"OPTIONS malformed JSON order changed")?;}}
        self.send("/api/sub/%FF",json!({"headers":{"Origin":null},"method":"OPTIONS","status":400,"tagged":false}))?;
        for singular in["sub","collection"]{let reply=self.send(&format!("/api/{singular}/missing-method-record"),json!({"method":"PATCH","body":{},"status":404}))?;require(Self::value(&reply)?["error"]["type"]=="ResourceNotFoundError","Missing PATCH error changed")?;}
        if self.origin.is_none(){for method in["POST","PUT","PATCH","DELETE","OPTIONS"]{self.send("/",json!({"method":method,"expected":"Hello from sub-store, made with ❤️ by Peng-YM"}))?;}self.send("/",json!({"method":"POST","raw":"{","status":400,"tagged":false}))?;}
        require(api::files(self.api)?==before,"HTTP method handling changed durable files")?;
        let before=api::files(self.api)?;
        for singular in["sub","collection"]{
            let target=format!("/api/{singular}/route-created");let mut expected=json!({"name":"route-created","marker":"created"});if singular=="collection"{expected["subscriptions"]=json!([]);}
            for(query,exported)in[("raw",false),("raw=",false),("raw=0",true),("raw=false",true),("raw=&raw=",true),("raw=1&raw=",true),("raw[]",true),("raw[x]=",true),("r%61w%5B%5D=",true),("raw%3Dx=1",false),("raw[x][y][z][u][v][w]=",true),("raw[__proto__]=",true)]{let reply=self.send(&format!("{target}?{query}"),json!({}))?;require(reply.headers.contains_key("content-disposition")==exported,"Typed raw query flag changed")?;let parsed=Self::value(&reply)?;json::check(if exported{&parsed}else{&parsed["data"]},&expected,false)?;}
            let mut modes=vec![("mode=invalid".into(),"invalid".into()),("mode=0".into(),"0".into()),("mode=archive&mode=permanent".into(),"archive,permanent".into()),("mode=permanent&mode=".into(),"permanent,".into()),("mode=&mode=".into(),",".into()),("mode[]=permanent".into(),"permanent".into()),("m%6Fde%5B%5D=permanent".into(),"permanent".into()),("mode[0]=permanent&mode[2]=".into(),"permanent,".into()),("mode[19]=permanent".into(),"permanent".into()),("mode=+permanent+".into()," permanent ".into()),("mode=%2570ermanent".into(),"%70ermanent".into()),("mode=%FF%20permanent".into(),"%FF%20permanent".into())];
            for query in["mode[20]=permanent","mode[x]=permanent","mode[x]=archive&mode=permanent","mode=lead&mode[20]=tail","mode[20]=tail&mode=toString","mode[x]=one&mode[20]=last&mode=toString","mode[20]=tail&mode[x]=one&mode=more","mode[x][y][z][u][v][w]=permanent","mode[__proto__]=permanent"]{modes.push((query.into(),"[object Object]".into()));}
            modes.extend([(vec!["mode=permanent";20].join("&"),vec!["permanent";20].join(",")),(vec!["mode=permanent";21].join("&"),"[object Object]".into()),(format!("mode=invalid&{}mode=permanent","x=&".repeat(999)),"invalid".into())]);
            for(query,mode)in modes{let reply=self.send(&format!("{target}?{query}"),json!({"method":"DELETE","status":500}))?;json::check(&Self::value(&reply)?,&json!({"status":"failed","error":{"code":"INVALID_DELETE_MODE","type":"RequestInvalidError","message":format!("Unsupported delete mode: {mode}")}}),false)?;}
            for query in["mode[toString]=not-callable","mode[x]=one&mode=toString","mode[][toString]=not-callable"]{let reply=self.send(&format!("{target}?{query}"),json!({"method":"DELETE","status":500}))?;json::check(&Self::value(&reply)?,&json!({"status":"failed","error":{"message":"Cannot convert object to primitive value"}}),false)?;}
            for(count,exported)in[(999,true),(1000,false)]{let reply=self.send(&format!("{target}?{}raw=1","x=&".repeat(count)),json!({}))?;require(reply.headers.contains_key("content-disposition")==exported,"Query parameter count boundary changed")?;}
        }require(api::files(self.api)?==before,"Query flags changed durable files")?;
        for(plural,singular)in[("subs","sub"),("collections","collection")]{for query in["","?mode=","?mode=%70ermanent"]{self.send(&format!("/api/{plural}"),json!({"method":"POST","body":{"name":"query-delete"},"status":201}))?;self.send(&format!("/api/{singular}/query-delete{query}"),json!({"method":"DELETE"}))?;require(self.api.durable()?[plural].as_array().ok_or("Missing query deletion list")?.iter().all(|item|item["name"]!="query-delete"),"Delete query did not remove target")?;}}
        Ok(())
    }
}
pub fn run(api:&Api,base:&str,origin:Option<&str>,output:&Path,label:&str)->Result<Vec<Value>>{
    let mut test=Entities{api,base,origin,output,label,observed:Vec::new()};test.run()?;json::write_new(output.join(format!("{label}.json")),&json!(test.observed))?;Ok(test.observed)
}
