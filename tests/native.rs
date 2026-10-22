use sub_store_tests::{Result,contracts,http::{self,Http},json,process,reference,require,server::{self,Backend,Server}};
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;

#[test]
fn frozen_node_unit_suite() -> Result { reference::unit(&process::scratch("reference-unit")?) }

#[test]
fn frozen_node_http_suites() -> Result { reference::http_suites(&process::scratch("reference-http")?) }

#[test]
fn rust_and_node_preserve_shared_state_restart_write_failure_and_rollback() -> Result {
    let output=process::scratch("state-parity")?;
    reference::verify(&output)?;
    let binary=PathBuf::from(std::env::var_os("TEST_RUST_BINARY").ok_or("Missing delivered Rust binary")?).canonicalize()?;
    let http=Http::new(Duration::from_secs(5))?; let cases=contracts::cases("service-state")?; let restart=contracts::cases("restart")?;
    let mut final_states=Vec::new();
    for (kind,backend) in [("node",Backend::Node(reference::node())),("rust",Backend::Rust(binary.clone()))] {
        let state=output.join(format!("{kind}-state")); std::fs::create_dir(&state)?; let mut environment=server::environment(&state);
        server::set(&mut environment,"SUB_STORE_BACKEND_PREFIX","true"); server::set(&mut environment,"SUB_STORE_FRONTEND_BACKEND_PATH","/backend");
        {
            let _listener=Server::start(&backend,&environment,&output,kind,false)?;
            contracts::check(&http,&_listener.url,&state,&cases,None,&output.join(format!("{kind}-contracts.json")))?;
            final_states.push(json::read(state.join("sub-store.json"))?);
        }
        {
            let label=format!("{kind}-restart"); let _listener=Server::start(&backend,&environment,&output,&label,false)?;
            contracts::check(&http,&_listener.url,&state,&restart,None,&output.join(format!("{kind}-restart.json")))?;
            if kind=="rust" {
                let previous=http.request(&_listener.url,&json!({"path":"/api/settings"}))?;
                let saved=state.join("saved-state.json"); let current=state.join("sub-store.json"); std::fs::rename(&current,&saved)?; std::fs::create_dir(&current)?;
                let result=(|| -> Result {
                    let failed=http.request(&_listener.url,&json!({"path":"/api/settings","method":"PATCH","body":{"mustNotCommit":true}}))?;
                    let unchanged=http.request(&_listener.url,&json!({"path":"/api/settings"}))?;
                    json::write_new(output.join("write-failure.json"),&json!({"before":previous,"failure":failed,"after":unchanged}))?;
                    require(failed["status"]==500 && json::parse(failed["body"].as_str().unwrap())?["error"]["code"]=="STATE_WRITE_FAILED","Failed state writes must return STATE_WRITE_FAILED")?;
                    json::check(&json::parse(unchanged["body"].as_str().unwrap())?,&json::parse(previous["body"].as_str().unwrap())?,false)
                })();
                std::fs::remove_dir(&current)?; std::fs::rename(&saved,&current)?; result?;
                let denied=http.request(&_listener.url,&json!({"path":"/api/settings","method":"PATCH","headers":{"Origin":"https://denied.invalid"},"body":{"mustNotCommit":true}}))?;
                http::check(&denied,&json!({"status":403,"absent_headers":["access-control-allow-origin"]}))?;
                require(http.request(&_listener.url,&json!({"path":"/api/storage","origin":true}))?["status"]==404,"Unprefixed API must be unavailable")?;
                require(http.request(&_listener.url,&json!({"path":"/api/settings","method":"OPTIONS","headers":{"Origin":"https://sub-store.vercel.app"}}))?["status"]==200,"CORS preflight failed")?;
            }
        }
    }
    json::check(&final_states[0],&final_states[1],false)?;
    let rollback=output.join("node-state"); std::fs::copy(output.join("rust-state/sub-store.json"),rollback.join("sub-store.json"))?;
    let mut environment=server::environment(&rollback); server::set(&mut environment,"SUB_STORE_BACKEND_PREFIX","true"); server::set(&mut environment,"SUB_STORE_FRONTEND_BACKEND_PATH","/backend");
    let _listener=Server::start(&Backend::Node(reference::node()),&environment,&output,"rollback",false)?;
    contracts::check(&http,&_listener.url,&rollback,&restart,None,&output.join("rollback.json"))?;
    Ok(())
}
