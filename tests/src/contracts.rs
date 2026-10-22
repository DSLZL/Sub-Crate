use crate::{Result, http::{self, Http}, json, providers, require, upstream::{Upstream,substitute}};
use base64::Engine;
use serde_json::{Map, Value, json};
use std::path::Path;

pub fn cases(suite: &str) -> Result<Vec<Value>> {
    json::load_cases(&json::read(crate::compat().join("manifest.json"))?,&json::read(crate::compat().join("fixtures/http-cases.json"))?,suite)
}

pub fn run(http: &Http, base: &str, state: Option<&Path>, cases: &[Value], upstream: Option<&Upstream>) -> Vec<Value> {
    let mut results=Vec::new();
    let mut captured=Map::new();
    for case in cases {
        let mut record=json!({"id":case["id"],"source":case["source"],"status":"passed","steps":[]});
        let result=(|| -> Result {
            if let Some(gists)=case.get("upstream_gists") {
                let provider=upstream.ok_or("Gist fixture requires controlled upstream")?;
                *provider.state.gists.lock().unwrap()=substitute(gists,provider.url());
            }
            for original in case["steps"].as_array().ok_or("Missing scenario steps")? {
                let step=upstream.map(|upstream| substitute(original,upstream.url())).unwrap_or_else(|| original.clone());
                let offset=upstream.map(|upstream| upstream.fixture.count()).unwrap_or(0);
                let sent=json::interpolate(&Value::Object(step.as_object().ok_or("Invalid step")?.iter().filter(|(key,_)| ["path","method","headers","body","origin"].contains(&key.as_str())).map(|(key,value)| (key.clone(),value.clone())).collect()),&captured)?;
                let response=http.request(base,&sent)?;
                let outgoing=upstream.map(|upstream| upstream.fixture.since(offset)).unwrap_or_default();
                record["steps"].as_array_mut().unwrap().push(json!({"method":sent.get("method").unwrap_or(&json!("GET")),"path":sent["path"],"origin":sent.get("origin").unwrap_or(&Value::Bool(false)),"response":response,"upstream_requests":outgoing}));
                let observed=record["steps"].as_array_mut().unwrap().last_mut().unwrap();
                if step.get("state").is_some() { observed["state"]=json::read(state.ok_or("State assertion requires state directory")?.join("sub-store.json"))?; }
                if step.get("upstream_state").is_some() { observed["upstream_state"]=upstream.ok_or("Provider assertion requires controlled upstream")?.gists(); }
                if let Some(backup)=step.get("base64_backup") {
                    let store=upstream.ok_or("Backup assertion requires controlled upstream")?.gists();
                    let content=store[backup["gist"].as_str().ok_or("Missing backup gist")?]["files"][backup["file"].as_str().ok_or("Missing backup file")?]["content"].as_str().ok_or("Missing provider backup content")?;
                    let raw=base64::engine::general_purpose::STANDARD.decode(content)?;
                    observed["base64_backup"]=json::parse(std::str::from_utf8(&raw)?)?;
                }
                if let Some(spec)=step.get("capture") {
                    let values=json::captures(&json::parse(response["body"].as_str().ok_or("Missing capture body")?)?,spec)?;
                    require(values.keys().all(|name| !captured.contains_key(name)),"Capture names must be unique")?;
                    observed["captured"]=json!(values); captured.extend(values);
                }
                if let Some(projection)=json::project_captured_scalars(&response,&step["expect"],&captured)? { observed["capture_projection"]=projection; }
                let step=json::interpolate(&step,&captured)?;
                http::check(&response,&step["expect"])?;
                if let Some(expected)=step.get("state") { json::check(&observed["state"],expected,true)?; }
                if let Some(expected)=step.get("upstream_state") { json::check(&observed["upstream_state"],expected,false)?; }
                if let Some(expected)=step.get("base64_backup") { json::check(&observed["base64_backup"],&expected["json"],false)?; }
                if step["gist_contracts"]==true {
                    providers::check_gist_requests(&outgoing)?;
                    observed["gist_state"]=upstream.ok_or("Gist assertion requires controlled upstream")?.state.script_gist.snapshot();
                    providers::check_gist_state(&observed["gist_state"])?;
                } else { json::check(&json!(outgoing),step.get("upstream").unwrap_or(&json!([])),true)?; }
            }
            Ok(())
        })();
        if let Err(error)=result { record["status"]=json!("failed"); record["error"]=json!(error.to_string()); }
        let failed=record["status"]=="failed";
        results.push(record);
        if failed { break; }
    }
    for case in &cases[results.len()..] { results.push(json!({"id":case["id"],"status":"not_run","reason":"Earlier scenario failed"})); }
    results
}

pub fn check(http: &Http, base: &str, state: &Path, cases: &[Value], upstream: Option<&Upstream>, output: &Path) -> Result<Vec<Value>> {
    let results=run(http,base,Some(state),cases,upstream);
    json::write_new(output,&json!(results))?;
    providers::require_complete(&results)?;
    Ok(results)
}

pub fn isolate_age_requests(cases: &mut [Value]) -> Result {
    let case=cases.iter_mut().find(|case| case["id"]=="PC-native-age-public-and-download").ok_or("Missing native age scenario")?;
    let steps=case["steps"].as_array().ok_or("Missing age steps")?; require(steps.len()==1,"Age source must have one original step")?;
    let original=steps[0].clone();
    let checks=original["expect"]["json_contains"]["data"]["processed"][0]["ageVerified"].as_array().ok_or("Missing crypto groups")?;
    let steps=checks.iter().map(|check| {
        let mut step=original.clone(); step["body"]["process"][0]["args"]["arguments"]["check"]=check.clone();
        step["expect"]["json_contains"]["data"]["processed"][0]["ageVerified"]=json!([check]);
        if check!="AG-native-download-auto-decrypt-and-original-cache" { step["upstream"]=json!([]); } step
    }).collect::<Vec<_>>();
    case["steps"]=json!(steps); Ok(())
}

pub fn compare_step(actual: &Value, reference: &Value) -> Result {
    let projected=actual.get("capture_projection"); let expected=reference.get("capture_projection");
    require(projected.is_some()==expected.is_some(),"Capture projection missing from a service")?;
    if let (Some(actual),Some(reference))=(projected,expected) { json::check(&actual["paths"],&reference["paths"],false)?; }
    require(actual["response"]["status"]==reference["response"]["status"],"Differential HTTP status mismatch")?;
    require(projected.unwrap_or(&actual["response"])["body"]==expected.unwrap_or(&reference["response"])["body"],"Differential raw body mismatch")?;
    for key in ["content-type","content-length","etag","allow"] {
        if projected.is_some() && ["content-length","etag"].contains(&key) { continue; }
        require(actual["response"]["headers"].get(key)==reference["response"]["headers"].get(key),format!("Differential header {key} mismatch"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn failed_steps_keep_responses_and_stop_dependent_scenarios() {
        let upstream=Upstream::new(0).unwrap(); let http=Http::new(Duration::from_secs(5)).unwrap();
        let cases=vec![json!({"id":"one","source":"self-check","steps":[{"path":"/subscription","expect":{"status":500}}]}),json!({"id":"two","source":"self-check","steps":[{"path":"/subscription","expect":{"status":200}}]})];
        let results=run(&http,upstream.url(),None,&cases,None);
        assert_eq!(results[0]["status"],"failed"); assert_eq!(results[0]["steps"][0]["response"]["status"],200);
        assert_eq!(results[1]["status"],"not_run"); assert_eq!(upstream.fixture.count(),1); upstream.fixture.healthy().unwrap();
    }

    #[test]
    fn upstream_requests_and_cache_assertions_preserve_types_and_null() {
        let upstream=Upstream::new(0).unwrap(); let http=Http::new(Duration::from_secs(5)).unwrap();
        let response=http.request(upstream.url(),&json!({"method":"HEAD","path":"/subscription"})).unwrap();
        assert_eq!(response["body"],""); assert_eq!(response["headers"]["subscription-userinfo"],"upload=10; download=20; total=1000");
        json::check(&json!(upstream.requests()),&json!([{"method":"HEAD","path":"/subscription","body":"","headers":{"content-type":"application/json"}}]),true).unwrap();
        for expected in [json!([]),json!([{"method":"GET"}]),json!([{"body":null}])] { assert!(json::check(&json!(upstream.requests()),&expected,true).is_err()); }
        assert_eq!(substitute(&json!({"url":"$UPSTREAM/subscription","nil":null}),upstream.url()),json!({"url":format!("{}/subscription",upstream.url()),"nil":null}));
    }

    #[test]
    fn age_requests_keep_every_crypto_group_and_upstream_assertion() {
        crate::actions_only();
        let mut cases=cases("conversion-local").unwrap();
        let original=cases.iter().find(|case| case["id"]=="PC-native-age-public-and-download").unwrap()["steps"][0].clone();
        assert_eq!(original["body"]["process"][0]["args"]["content"].as_str().unwrap().replace("\r\n","\n"),std::fs::read_to_string(crate::compat().join("fixtures/script-age-native.js")).unwrap().replace("\r\n","\n"));
        let checks=original["expect"]["json_contains"]["data"]["processed"][0]["ageVerified"].as_array().unwrap();
        isolate_age_requests(&mut cases).unwrap();
        let split=cases.iter().find(|case| case["id"]=="PC-native-age-public-and-download").unwrap()["steps"].as_array().unwrap();
        assert_eq!(checks.len(),split.len()); let mut outgoing=Vec::new();
        for (check,step) in checks.iter().zip(split) {
            assert_eq!(&step["body"]["process"][0]["args"]["arguments"]["check"],check);
            assert_eq!(step["expect"]["json_contains"]["data"]["processed"][0]["ageVerified"],json!([check]));
            outgoing.extend(step["upstream"].as_array().unwrap().clone());
            let mut restored=step.clone(); restored["body"]["process"][0]["args"]["arguments"].as_object_mut().unwrap().remove("check"); restored["expect"]=original["expect"].clone(); restored["upstream"]=original["upstream"].clone(); assert_eq!(restored,original);
        }
        assert_eq!(json!(outgoing),original["upstream"]);
    }
}
