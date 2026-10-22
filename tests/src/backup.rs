use crate::{Result,api::Api,contracts,json,reference,require,service,server::Backend,upstream::Upstream};
use serde_json::{Value,json};
use std::path::Path;

pub fn age_interop(output:&Path)->Result {
    reference::verify(output)?;let (baseline,root)=service::baseline(output)?;
    let secret=contracts::cases("utilities-core")?.into_iter().find(|case|case["id"]=="Z01-age-rest-key-generation-and-validation").ok_or("Missing age key scenario")?["steps"][0]["body"]["age-secret-key"].clone();
    let secret=secret.as_str().ok_or("Missing age key fixture")?;let upstream=Upstream::new(0)?;let mut errors=Vec::new();
    for (kind,phase,backend) in [("node","upload",Backend::Node(reference::node())),("rust","cross",service::target()?),("node","cross",Backend::Node(reference::node()))] {
        let label=format!("{kind}-backup-age-{phase}");let mut document=baseline.clone();document["settings"]=json!({"logsMaxCount":1000,"gistToken":"helper-token","githubApiUrl":format!("{}/host/gist/github",upstream.url()),"gistUpload":"age","age-secret-key":secret});document["files"]=json!([{"name":"age-backup-original","source":"local","content":"保留🙂\r\n"}]);for field in ["artifacts","tokens","archives","modules"] {document[field]=json!([]);}document["ageBackup"]=json!({"nil":null,"zero":0,"flag":false,"script":"require('node:fs')\r\n// stored only"});
        let mut root=root.clone();root["sub-store-logs"]=json!("[]");let (state,environment)=service::seed(output,&label,&document,&root)?;let api=Api::start(&backend,&environment,&state,output,&label)?;
        if phase=="cross" {api.send(&json!({"path":"/api/utils/backup?action=download"}),200)?;let restored=api.durable()?;for field in ["ageBackup","subs"] {json::check(&restored[field],&document[field],false)?;}json::check(&restored["files"][0],&document["files"][0],false)?;require(restored["settings"]["age-secret-key"]==secret,"Restored backup key changed")?;if kind=="node" {json::check(&restored["files"][1],&json!({"name":"from-rust","source":"local","content":"Rust 备份🙂"}),false)?;}}
        if kind=="rust" {api.send(&json!({"method":"POST","path":"/api/files","body":{"name":"from-rust","source":"local","content":"Rust 备份🙂"}}),201)?;}
        if phase=="upload" || kind=="rust" {
            api.send(&json!({"path":"/api/utils/backup?action=upload"}),200)?;let saved=upstream.state.script_gist.snapshot();let (id,backup)=saved["github"].as_object().ok_or("Missing uploaded backup")?.iter().find(|(_,value)|value["description"]=="Auto Generated Sub-Store Backup").ok_or("Missing backup gist")?;
            let ciphertext=backup["files"]["Sub-Store"]["content"].as_str().ok_or("Missing backup armor")?;require(ciphertext.starts_with("-----BEGIN AGE ENCRYPTED FILE-----") && !ciphertext.contains(secret) && !ciphertext.contains("保留🙂"),"Encrypted backup exposed plaintext")?;
            let before=std::fs::read(state.join("sub-store.json"))?;let mut root_before=api.stored("root.json")?;root_before.as_object_mut().ok_or("Invalid root state")?.shift_remove("sub-store-logs");
            let mut corrupt=saved.clone();corrupt["github"][id]["files"]["Sub-Store"]["content"]=json!("-----BEGIN AGE ENCRYPTED FILE-----\ncorrupted\n-----END AGE ENCRYPTED FILE-----");upstream.state.script_gist.restore(&corrupt)?;
            let checked=(||->Result<Value> {let error=api.send(&json!({"path":"/api/utils/backup?action=download"}),500)?;require(std::fs::read(state.join("sub-store.json"))?==before,"Failed decryption changed durable data")?;let mut root_after=api.stored("root.json")?;root_after.as_object_mut().ok_or("Invalid root state")?.shift_remove("sub-store-logs");json::check(&root_after,&root_before,false)?;Ok(error)})();upstream.state.script_gist.restore(&saved)?;errors.push(checked?);api.send(&json!({"path":"/api/logs?limit=10"}),200)?;
        }
        json::write_new(output.join(format!("{label}-final.json")),&json!({"data":api.durable()?,"root":api.stored("root.json")?,"provider":upstream.state.script_gist.snapshot(),"upstream":upstream.fixture.snapshot()}))?;
    }
    require(errors.len()==2,"Missing corrupted backup errors")?;json::check(&errors[0],&errors[1],false)
}
