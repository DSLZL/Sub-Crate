use crate::{Result,contracts,http::Http,json,process,require,server::{self,Backend,Server},upstream::Upstream};
use serde_json::{Value,json};
use std::path::{Path,PathBuf};
use std::process::Command;
use std::time::Duration;

pub fn node() -> PathBuf { std::env::var_os("TEST_NODE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("node")) }

pub fn verify(output: &Path) -> Result<Value> {
    let provenance=process::provenance(output)?;
    let version=process::checked(Command::new(node()).arg("--version"),output,"node-version",Duration::from_secs(30))?;
    require(version.stdout.trim()=="v24.16.0","Pinned Node v24.16.0 required")?;
    require(crate::root().join("backend/dist/sub-store.bundle.js").is_file(),"Missing prepared Node reference bundle")?;
    Ok(provenance)
}

pub fn unit(output: &Path) -> Result {
    let mut report=verify(output)?;
    let loader=if cfg!(windows) { crate::compat().join("register.cjs").to_string_lossy().into_owned() } else { "@babel/register".to_owned() };
    let result=process::run(Command::new(node()).arg(crate::root().join("backend/node_modules/mocha/bin/_mocha")).args(["src/test/**/*.spec.js","--require",&loader,"--require"]).arg(crate::compat().join("unit-progress.cjs")).args(["--recursive","--reporter","json"]).current_dir(crate::root().join("backend")),output,"mocha",Duration::from_secs(900))?;
    let start=regex::Regex::new(r#"\{\s*"stats"\s*:"#)?.find(&result.stdout).ok_or("Mocha JSON result missing; inspect preserved progress logs")?.start();
    let mut stream=serde_json::Deserializer::from_str(&result.stdout[start..]).into_iter::<Value>();
    let mocha=stream.next().ok_or("Empty Mocha JSON result")??;
    report["stats"]=mocha["stats"].clone(); report["pending"]=mocha["pending"].clone(); report["failures"]=mocha["failures"].clone();
    json::write_new(output.join("node-unit.json"),&report)?;
    require(result.success() && mocha["stats"]["tests"]==826 && mocha["pending"].as_array().is_some_and(Vec::is_empty) && mocha["failures"].as_array().is_some_and(Vec::is_empty),"Frozen Node unit suite did not pass all 826 tests without pending cases")
}

pub fn http_suites(output: &Path) -> Result {
    let provenance=verify(output)?; json::write_new(output.join("provenance.json"),&provenance)?;
    let state=output.join("state"); std::fs::create_dir(&state)?;
    let http=Http::new(Duration::from_secs(5))?;
    let phases=["smoke","restart","merged-share","conversion","resources","operations","operations-restart","cors-configured","cors-wildcard","cors-branded","gist","node-info"];
    for suite in phases {
        let upstream=Upstream::new(0)?; let mut environment=server::environment(&state);
        match suite {
            "merged-share"=>{ server::set(&mut environment,"SUB_STORE_BACKEND_MERGE","true"); server::set(&mut environment,"SUB_STORE_FRONTEND_BACKEND_PATH","/backend"); },
            "cors-configured"=>{ server::set(&mut environment,"SUB_STORE_CORS_ALLOWED_ORIGINS","https://allowed.invalid/path, https://second.invalid"); server::set(&mut environment,"SUB_STORE_BACKEND_CUSTOM_NAME","fixture-server"); },
            "cors-wildcard"=>server::set(&mut environment,"SUB_STORE_CORS_ALLOWED_ORIGINS","*"),
            "cors-branded"=>{ for (key,value) in [("SUB_STORE_BACKEND_CUSTOM_NAME","fixture-server"),("SUB_STORE_BACKEND_CUSTOM_ICON","https://icon.invalid/test.png"),("SUB_STORE_DOCKER","true"),("SUB_STORE_CRON","0 0 * * *")] { server::set(&mut environment,key,value); } },
            "node-info"=>{ for key in ["HTTP_PROXY","http_proxy","HTTPS_PROXY","https_proxy"] { server::set(&mut environment,key,upstream.url()); } for key in ["ALL_PROXY","all_proxy"] { server::set(&mut environment,key,""); } for key in ["NO_PROXY","no_proxy"] { server::set(&mut environment,key,"127.0.0.1,localhost"); } },
            _=>(),
        }
        let _listener=Server::start(&Backend::Node(node()),&environment,output,suite,false)?;
        contracts::check(&http,&_listener.url,&state,&contracts::cases(suite)?,Some(&upstream),&output.join(format!("{suite}.json")))?;
        upstream.fixture.healthy()?;
    }
    Ok(())
}
