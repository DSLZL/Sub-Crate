use super::*;
use base64::{Engine,engine::general_purpose::STANDARD};
use std::{path::{Component,PathBuf},process::Command,time::Duration};

pub fn cipher(operation:&str,node:&Path,input_file:&Path,body:Vec<u8>,output:&Path)->Result<Vec<u8>>{
    check(["seal","open"].contains(&operation),"capture")?;let mut command=Command::new(node);command.args([crate::root().join("tests/performance/frozen-inputs.cjs").as_os_str(),operation.as_ref(),input_file.as_os_str()]);
    for(key,_)in std::env::vars_os().filter(|(key,_)|key.to_string_lossy().starts_with("BENCHMARK_")){command.env_remove(key);}
    transport::pipe(&mut command,body,output,operation,Duration::from_secs(120))
}
pub fn seal(node:&Path,input_file:&Path,scratch:&Path,pristine:&BTreeMap<String,PathBuf>,report:&Value,destination:&Path,output:&Path)->Result{
    let mut files=Vec::new();for(prefix,directory)in pristine.iter().map(|(kind,directory)|(format!("pristine-{kind}"),directory)).chain(std::iter::once(("mirrors".to_owned(),&scratch.join("mirrors")))){
        check(prefix=="mirrors"||["pristine-node","pristine-rust"].contains(&prefix.as_str()),"capture")?;
        for path in crate::distribution::asset_hashes(directory)?.keys(){let bytes=std::fs::read(directory.join(path))?;files.push(json!({"path":format!("{prefix}/{path}"),"data":STANDARD.encode(bytes)}));}
    }
    let body=json!({"version":2,"prepared":report,"files":files});let encrypted=cipher("seal",node,input_file,canonical(&body),output)?;std::fs::write(destination,encrypted)?;Ok(())
}
fn destination(root:&Path,path:&str)->Result<PathBuf>{
    check(!path.is_empty()&&!path.contains(['\\','\0'])&&!Path::new(path).is_absolute()&&Path::new(path).components().all(|part|matches!(part,Component::Normal(_))),"capture")?;
    check(path.starts_with("mirrors/")||path.starts_with("pristine-node/")||path.starts_with("pristine-rust/"),"capture")?;Ok(root.join(path))
}
pub fn open(node:&Path,input_file:&Path,source:&Path,scratch:&Path,current:&Value,profile:bool,output:&Path)->Result<Value>{
    let bytes=std::fs::read(source)?;check(bytes.len()<=512*1024*1024,"capture")?;let body=cipher("open",node,input_file,bytes,output)?;let body:Value=serde_json::from_slice(&body)?;check(body["version"]==2,"capture")?;let prepared=&body["prepared"];
    check(prepared["preparation_complete"]==true&&prepared["input_sha256"]==INPUT_HASH&&prepared["native_pipeline_profile"].as_bool().unwrap_or(false)==profile,"capture")?;
    let provenance=current["provenance"].as_object().ok_or("Current provenance missing")?;check(provenance.iter().filter(|(key,_)|key.as_str()!="runner").all(|(key,value)|prepared["provenance"][key]==*value),"source")?;
    let imports=prepared["imports"].as_array().ok_or("Prepared imports missing")?;check(imports.len()==10&&imports.iter().all(|row|row["restart"]==true)&&KINDS.into_iter().all(|kind|imports.iter().filter(|row|row["kind"]==kind).count()==5),"persistence")?;
    let sources=prepared["capture"]["sources"].as_array().ok_or("Capture sources missing")?;check(prepared["capture"]["before_preprocessing"]==true&&sources.len()==22&&sources.iter().filter_map(|row|row["index"].as_u64()).collect::<BTreeSet<_>>()==(0..22).collect(),"capture")?;
    let mut paths=BTreeSet::new();let mut files=Vec::new();for file in body["files"].as_array().ok_or("Capsule files missing")?{let path=file["path"].as_str().ok_or("Capsule path missing")?;let target=destination(scratch,path)?;check(paths.insert(path.to_owned()),"capture")?;let bytes=STANDARD.decode(file["data"].as_str().ok_or("Capsule bytes missing")?)?;files.push((target,bytes));}
    for(target,bytes)in files{std::fs::create_dir_all(target.parent().ok_or("Capsule parent missing")?)?;use std::io::Write;std::fs::OpenOptions::new().write(true).create_new(true).open(target)?.write_all(&bytes)?;}
    for row in sources{let path=scratch.join(format!("mirrors/{}.txt",row["index"].as_u64().unwrap()));check(json!(digest(std::fs::read(path)?))==row["sha256"],"capture")?;}
    for kind in KINDS{check(scratch.join(format!("pristine-{kind}/sub-store.json")).is_file()&&scratch.join(format!("pristine-{kind}/root.json")).is_file(),"persistence")?;}Ok(prepared.clone())
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn authenticated_capsule_rejects_changed_input_run_public_digest_and_private_plaintext()->Result{
        let root=crate::process::scratch("performance-cipher")?;
        let script=r#"const assert=require('node:assert/strict'),{createHash}=require('node:crypto');
const {seal,open}=require('./tests/performance/frozen-inputs.cjs');
const input=Buffer.from('private-subscription-'.repeat(100)),plain=Buffer.from('private-raw-yaml\r\n节点\0private-seed');
const encrypted=seal(input,plain,'123');assert(open(input,encrypted,'123').equals(plain));assert(!encrypted.includes(plain));
const changed=Buffer.from(encrypted);changed[changed.length-1]^=1;
for(const [password,body,run] of [[input,changed,'123'],[input,encrypted,'124'],[Buffer.from('wrong'),encrypted,'123'],[createHash('sha256').update(input).digest(),encrypted,'123'],[createHash('sha256').update(input).digest('hex'),encrypted,'123']])assert.throws(()=>open(password,body,run));"#;
        check(crate::process::run(std::process::Command::new(crate::reference::node()).args(["-e",script]).current_dir(crate::root()),&root,"authentication",Duration::from_secs(90))?.success(),"capture")
    }
    #[test]
    fn encrypted_state_roundtrip_keeps_raw_bytes_and_rejects_wrong_capture_or_traversal()->Result{
        let root=crate::process::scratch("performance-capsule")?;let input=root.join("private-input.json");std::fs::write(&input,b"private-input".repeat(100))?;let sender=root.join("sender");std::fs::create_dir(&sender)?;let mirrors=sender.join("mirrors");std::fs::create_dir(&mirrors)?;
        let mut sources=Vec::new();for index in 0..22{let raw=if index==3{String::new()}else{format!("private-{index}\r\n节点")};std::fs::write(mirrors.join(format!("{index}.txt")),raw.as_bytes())?;sources.push(json!({"index":index,"sha256":digest(raw.as_bytes())}));}
        let mut pristine=BTreeMap::new();for kind in KINDS{let state=sender.join(format!("pristine-{kind}"));std::fs::create_dir(&state)?;for name in["root.json","sub-store.json"]{std::fs::write(state.join(name),b"private-ui-created-state")?;}pristine.insert(kind.to_owned(),state);}
        let mut report=json!({"status":"passed","preparation_complete":true,"input_sha256":INPUT_HASH,"chromium":"1.2.3","provenance":{"harness_sha":"a".repeat(40)},"imports":KINDS.into_iter().flat_map(|kind|(0..5).map(move |_|json!({"kind":kind,"restart":true}))).collect::<Vec<_>>(),"capture":{"before_preprocessing":true,"sources":sources}});let capsule=root.join("frozen.bin");
        for corrupt in[false,true]{if corrupt{report["capture"]["sources"][2]["sha256"]=json!("d".repeat(64));}let send=root.join(format!("send-{corrupt}"));std::fs::create_dir(&send)?;seal(&crate::reference::node(),&input,&sender,&pristine,&report,&capsule,&send)?;let receive=root.join(format!("receive-{corrupt}"));std::fs::create_dir(&receive)?;let current=json!({"provenance":report["provenance"]});let result=open(&crate::reference::node(),&input,&capsule,&receive,&current,false,&receive);
            if corrupt{assert!(result.is_err());}else{let prepared=result?;assert_eq!(prepared["capture"],report["capture"]);for kind in KINDS{assert_eq!(std::fs::read(receive.join(format!("pristine-{kind}/root.json")))?,b"private-ui-created-state");}assert_eq!(std::fs::read(receive.join("mirrors/0.txt"))?,std::fs::read(mirrors.join("0.txt"))?);}
        }
        for path in["../escape","/absolute","mirrors/../../escape","mirrors\\escape","other/file"]{assert!(destination(&root,path).is_err());}Ok(())
    }
}
