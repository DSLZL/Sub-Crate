use crate::{Result, api::{self,Api}, fixture::quote, json, media, raw_http, reference, require, server::{self, Backend}, service};
use base64::{Engine,engine::general_purpose::STANDARD};
use serde_json::{Value,json};
use std::{collections::BTreeMap,path::Path};

fn units(text: &str) -> Result<Vec<u16>> {
    let mut units=Vec::new();let mut chars=text.chars();
    while let Some(character)=chars.next() {
        if character=='\u{e000}' {
            if chars.as_str().starts_with('\u{e000}') {chars.next(); units.push(0xe000);} else {units.push(u16::from_str_radix(&chars.by_ref().take(4).collect::<String>(),16)?);}
        } else {units.extend(character.encode_utf16(&mut [0;2]).iter().copied());}
    }Ok(units)
}
fn utf16_json(raw: &str) -> Result<Vec<u8>> {
    let mut result=Vec::new();let mut chars=raw.chars();
    while let Some(character)=chars.next() {
        if character=='\\' {
            let escaped=chars.next().ok_or("Incomplete JSON escape")?;
            if escaped=='u' {
                let digits=chars.by_ref().take(4).collect::<String>();let word=u16::from_str_radix(&digits,16)?;
                if (0xd800..=0xdfff).contains(&word) {result.extend(word.to_le_bytes());continue;}
                for word in format!("\\u{digits}").encode_utf16(){result.extend(word.to_le_bytes());}
            }else{for word in ['\\',escaped].iter().flat_map(|character|character.encode_utf16(&mut [0;2]).to_vec()){result.extend(word.to_le_bytes());}}
        }else{for word in character.encode_utf16(&mut [0;2]){result.extend(word.to_le_bytes());}}
    }Ok(result)
}
struct Unicode<'a>{api:&'a Api,output:&'a Path,label:&'a str,observed:Vec<Value>}
impl Unicode<'_>{
    fn send(&mut self,method:&str,path:&str,body:Option<Value>,status:u16,utf16:bool)->Result<Value>{
        let before=api::files(self.api)?;let mut headers=BTreeMap::new();
        let encoded=if let Some(body)=body{headers.insert("Content-Type".into(),format!("application/json; charset={}",if utf16{"utf-16le"}else{"utf-8"}));let raw=json::stringify(&body);if utf16{utf16_json(&raw)?}else{raw.into_bytes()}}else{Vec::new()};
        let reply=raw_http::request(&self.api.listener.url,method,path,&headers,&encoded,None)?;let after=api::files(self.api)?;
        let state=self.api.durable()?;let root=self.api.stored("root.json")?;
        json::write_new(self.output.join(format!("{}-{:04}.json",self.label,self.observed.len())),&json!({"method":method,"path":path,"status":reply.status,"headers":reply.headers,"request_headers":headers,"request_base64":STANDARD.encode(encoded),"body_base64":STANDARD.encode(&reply.body),"state":state,"root":root,"files_before_base64":before.iter().map(|bytes|STANDARD.encode(bytes)).collect::<Vec<_>>(),"files_after_base64":after.iter().map(|bytes|STANDARD.encode(bytes)).collect::<Vec<_>>()}))?;
        require(reply.status==status,format!("Unicode response changed: {method} {path} {}",reply.evidence()))?;
        if method=="GET"||status>=400{require(before==after,"Unicode read/rejected request changed durable files")?;}
        let length=reply.body.len().to_string();require(reply.headers.get("content-length")==Some(&length),"Unicode byte length changed")?;
        if let Some(tag)=reply.headers.get("etag"){require(tag==&media::etag(&reply.body),"Unicode ETag differs from actual bytes")?;}
        self.observed.push(json!({"method":method,"path":path,"status":status,"body_base64":STANDARD.encode(&reply.body),"state":state,"root":root}));
        json::parse(std::str::from_utf8(&reply.body)?)
    }
    fn run(&mut self,mutate:bool)->Result{
        let original=self.api.durable()?;
        json::check(&self.send("GET","/api/storage",None,200,false)?,&original,false)?;
        json::check(&self.send("GET","/api/settings",None,200,false)?["data"],&original["settings"],false)?;
        if mutate{
            let words=units(original["settings"].as_str().ok_or("Missing Unicode string settings")?)?;
            let mut expected=Value::Object(words.iter().enumerate().map(|(index,word)|Ok((index.to_string(),json::parse(&format!("\"\\u{word:04x}\""))?))).collect::<Result<serde_json::Map<_,_>>>()?);
            json::check(&self.send("PATCH","/api/settings",Some(json!({})),200,false)?["data"],&expected,false)?;
            let values=json::parse(r#"{"high":"\ud800","low":"\udfff","pair":"🙂","literal":"\\ud800","marker":"\ue000d800","markers":"\ue000\ue000","\ud800":"\udc00","nil":null}"#)?;
            expected["unicode"]=values.clone();
            for utf16 in [false,true]{json::check(&self.send("PATCH","/api/settings",Some(json!({"unicode":values})),200,utf16)?["data"],&expected,false)?;}
            let mut item=json::parse(r#"{"name":"unicode-\ue000d800-🙂"}"#)?;item["unicode"]=values.clone();
            json::check(&self.send("POST","/api/subs",Some(item.clone()),201,false)?["data"],&item,false)?;
            // Fixture text has a literal private-use character, while the internal codec doubles it.
            let name="unicode-\u{e000}d800-🙂";let path=format!("/api/sub/{}",quote(name));
            json::check(&self.send("GET",&path,None,200,false)?["data"],&item,false)?;
            json::check(&self.send("GET",&format!("{path}?raw=1"),None,200,false)?,&item,false)?;
            let mut collection=item.clone();collection["subscriptions"]=json!([item["name"]]);self.send("POST","/api/collections",Some(collection),201,false)?;
            let mut restored=self.api.durable()?;restored["unknown"]=values;
            let raw=json::stringify(&restored);
            for content in [raw.clone(),STANDARD.encode(raw)]{
                self.send("POST","/api/storage",Some(json!({"content":content})),200,false)?;
                json::check(&self.send("GET","/api/storage",None,200,false)?,&restored,false)?;
                json::check(&json::parse(self.api.stored("root.json")?["sub-store"].as_str().ok_or("Missing Unicode backup root")?)?,&restored,false)?;
            }
            restored["settings"]=Value::Null;self.send("POST","/api/storage",Some(json!({"content":json::stringify(&restored)})),500,false)?;
        }
        for plural in ["subs","collections"]{json::check(&self.send("GET",&format!("/api/{plural}"),None,200,false)?["data"],&self.api.durable()?[plural],false)?;}
        json::check(&self.send("GET","/api/settings",None,200,false)?["data"],&self.api.durable()?["settings"],false)
    }
}
pub fn run(output:&Path)->Result{
    reference::verify(output)?;let mut paired=Vec::new();let mut rust_state=None;
    for(kind,backend)in[("node",Backend::Node(reference::node())),("rust",service::target()?)]{
        let label=format!("{kind}-unicode");let mut document=json::read(crate::compat().join("fixtures/frontend-backup.json"))?;
        document["subs"]=json!([]);document["collections"]=json!([]);document["settings"]=json::parse(r#""A中🙂\ue000d800\ud800""#)?;
        document["unknown"]=json::parse(r#"{"\udfff":"\ud800","literal":"\ue000d800"}"#)?;
        let root=json!({"opaque":{"text":"cache🙂","zero":0,"flag":false},"sub-store-cached-resource":"{}","unicodeRoot":document["unknown"]});
        let(state,mut env)=service::seed(output,&label,&document,&root)?;server::set(&mut env,"NODE_ENV","production");let mut observed=Vec::new();
        for(phase,mutate)in[("live",true),("restart",false)]{
            let label=format!("{label}-{phase}");let api=Api::start(&backend,&env,&state,output,&label)?;
            let mut test=Unicode{api:&api,output,label:&label,observed:Vec::new()};test.run(mutate)?;
            json::write_new(output.join(format!("{label}.json")),&json!(test.observed))?;observed.extend(test.observed);
        }
        if kind=="rust"{rust_state=Some(state);}paired.push(observed);
    }
    json::check(&json!(paired[0]),&json!(paired[1]),false)?;
    let state=output.join("unicode-node-rollback");std::fs::create_dir(&state)?;for file in["sub-store.json","root.json"]{std::fs::copy(rust_state.as_ref().ok_or("Missing Unicode Rust state")?.join(file),state.join(file))?;}
    let mut env=server::environment(&state);server::set(&mut env,"NODE_ENV","production");let api=Api::start(&Backend::Node(reference::node()),&env,&state,output,"unicode-node-rollback")?;
    let mut test=Unicode{api:&api,output,label:"unicode-node-rollback",observed:Vec::new()};test.run(false)?;
    json::check(&json!(test.observed),&json!(paired[0][paired[0].len()-test.observed.len()..]),false)
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn utf16_fixture_keeps_lone_units_literal_escapes_markers_and_pairs_distinct()->Result{
        crate::actions_only();let value=json::parse(r#""A中🙂\ue000d800\ud800""#)?;
        require(units(value.as_str().unwrap())?==[0x41,0x4e2d,0xd83d,0xde42,0xe000,0x64,0x38,0x30,0x30,0xd800],"Unicode fixture unit order changed")?;
        require(utf16_json(r#"{"high":"\ud800","literal":"\\ud800","control":"\u0000"}"#)?==["{\"high\":\"".encode_utf16().collect::<Vec<_>>(),vec![0xd800],"\",\"literal\":\"\\\\ud800\",\"control\":\"\\u0000\"}".encode_utf16().collect()].concat().iter().flat_map(|word|word.to_le_bytes()).collect::<Vec<_>>(),"UTF16 wire fixture changed JSON controls/literal escape or isolated unit")
    }
}
