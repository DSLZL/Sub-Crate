use super::*;
use crate::{process,server::{self,Environment}};
use base64::{Engine,engine::general_purpose::STANDARD};
use std::{path::PathBuf,process::Command,time::{Duration,Instant,SystemTime}};

pub struct Arguments{pub original:PathBuf,pub rust:PathBuf,pub baseline:Option<PathBuf>,pub frontend:PathBuf,pub scratch:PathBuf,pub publish:PathBuf,pub diagnostic:Option<String>,pub profile:bool}
pub struct Owned{pub process:process::Running}
impl Owned{
    pub fn spawn(command:&mut Command,output:&Path,label:&str)->Result<Self>{
        #[cfg(unix)]{use std::os::unix::process::CommandExt;command.process_group(0);}
        let process=process::Running::spawn(command,output,label)?;write(output.join(format!("{label}.ownership.json")),&json!({"pid":process.child.id(),"direct_reason":"Measure and clean the actual process group without a wrapper"}))?;Ok(Self{process})
    }
    pub fn pid(&self)->u32{self.process.child.id()}
}
impl Drop for Owned{fn drop(&mut self){#[cfg(unix)]{unsafe{libc::kill(-(self.pid()as libc::pid_t),libc::SIGTERM);}}let _=self.process.stop();#[cfg(unix)]{unsafe{libc::kill(-(self.pid()as libc::pid_t),libc::SIGKILL);}}}}
pub struct Application{pub owned:Owned,pub origin:String,pub state:PathBuf,pub startup:f64,pub directory:PathBuf}
pub struct Suite{
    pub args:Arguments,pub report:Value,pub environment:Environment,pub node:PathBuf,pub pristine:BTreeMap<String,PathBuf>,pub expected:BTreeMap<String,Value>,pub expected_hashes:BTreeMap<String,String>,pub snapshot:Option<Value>,pub snapshot_hash:Option<String>,pub clash_hash:Option<String>,
    pub fixture:Option<Owned>,pub fixture_origin:String,pub meta:Option<Owned>,ordinal:u64,
}
pub fn clean_environment()->Environment{
    let mut env=std::env::vars_os().filter(|(key,_)|!["SUB_STORE_","AGENT_BROWSER_","BENCHMARK_","HTTP_PROXY","HTTPS_PROXY","ALL_PROXY"].iter().any(|prefix|key.to_string_lossy().starts_with(prefix))).collect::<Environment>();
    for(key,value)in[("NODE_ENV","production"),("TZ","UTC"),("NO_PROXY","127.0.0.1,localhost")]{server::set(&mut env,key,value);}env
}
fn ready(child:&mut Owned,url:&str)->Result{
    let started=Instant::now();while started.elapsed()<Duration::from_secs(40)&&child.process.child.try_wait()?.is_none(){if transport::http(url,"GET",None).is_ok(){return Ok(());}std::thread::sleep(Duration::from_millis(50));}check(false,"startup")
}
impl Suite{
    pub fn new(args:Arguments,report:Value)->Result<Self>{crate::actions_only();Ok(Self{args,report,environment:clean_environment(),node:crate::reference::node(),pristine:BTreeMap::new(),expected:BTreeMap::new(),expected_hashes:BTreeMap::new(),snapshot:None,snapshot_hash:None,clash_hash:None,fixture:None,fixture_origin:String::new(),meta:None,ordinal:0})}
    pub fn directory(&mut self,label:&str)->Result<PathBuf>{self.ordinal+=1;let path=self.args.scratch.join(format!("{}-{label}",self.ordinal));std::fs::create_dir(&path)?;Ok(path)}
    pub fn command(&mut self,command:&mut Command,label:&str,timeout:u64)->Result<String>{let output=self.directory(label)?;Ok(process::checked(command,&output,"command",Duration::from_secs(timeout))?.stdout)}
    pub fn server(&self,kind:&str,directory:PathBuf,clone:bool)->Result<Application>{
        let state=directory.join("state");if clone{crate::distribution::copy_tree(self.pristine.get(if kind=="node"{"node"}else{"rust"}).ok_or("Missing UI-created repetition seed")?,&state)?;}else{std::fs::create_dir(&state)?;}
        let port=server::free_port()?;let origin=format!("http://127.0.0.1:{port}");let mut env=self.environment.clone();
        for(key,value)in[("SUB_STORE_DATA_BASE_PATH",state.as_os_str().to_owned()),("SUB_STORE_BACKEND_API_HOST","127.0.0.1".into()),("SUB_STORE_BACKEND_API_PORT",port.to_string().into()),("SUB_STORE_FRONTEND_PATH",self.args.frontend.join("dist").into_os_string()),("SUB_STORE_FRONTEND_BACKEND_PATH","/backend".into()),("SUB_STORE_BACKEND_MERGE","true".into()),("SUB_STORE_CORS_ALLOWED_ORIGINS",origin.clone().into())]{server::set(&mut env,key,value);}
        let mut command=if kind=="node"{let mut command=Command::new(&self.node);command.arg(self.args.original.join("backend/dist/sub-store.bundle.js")).current_dir(self.args.original.join("backend"));command}else{
            check(["rust","rust_baseline","rust_repeat"].contains(&kind),"input")?;let root=if kind=="rust_baseline"{self.args.baseline.as_ref().ok_or("Missing distinct Rust baseline")?}else{&self.args.rust};server::set(&mut env,"PATH","");if kind=="rust_repeat"&&self.args.profile{server::set(&mut env,"SUB_STORE_PROFILE","1");}let mut command=Command::new(root.join("rust/target/release/sub-store-rust"));command.current_dir(root);command
        };
        command.env_clear().envs(&env);let began=Instant::now();let mut owned=Owned::spawn(&mut command,&directory,"server")?;ready(&mut owned,&format!("{origin}/backend/api/utils/env"))?;Ok(Application{owned,origin,state,startup:began.elapsed().as_secs_f64(),directory})
    }
    pub fn publish(&mut self)->Result{report::publish(&mut self.report,&self.args.publish)}
    pub fn configure(&self,origin:&str,original:&Value,mirrored:bool,variant:&str)->Result<Value>{
        let stored=transport::api(origin,"subs","GET",None)?;check(stored.as_array().is_some_and(|items|items.len()==1),"import_fields")?;
        let config=if mirrored{configured(original,&self.fixture_origin,variant)?}else{original.clone()};let name=stored[0]["name"].as_str().ok_or("Imported name missing")?;transport::api(origin,&format!("sub/{}",crate::fixture::quote(name)),"PATCH",Some(&config))?;Ok(config)
    }
    pub fn download(&self,origin:&str,name:&str,target:&str)->Result<Vec<u8>>{transport::http(&format!("{origin}/backend/download/{}?target={target}",crate::fixture::quote(name)),"GET",None)}
    pub fn imports(&mut self,original:&Value,input_file:&Path)->Result{
        let pattern=regex::Regex::new(r"(?:HeadlessChrome|Chrome)/([\d.]+)")?;
        for block in 0..5{let order=if block%2==0{KINDS}else{["rust","node"]};for kind in order{
            let directory=self.directory(&format!("import-{kind}"))?;let app=self.server(kind,directory.clone(),false)?;let mut browser=crate::browser::Browser::new(&directory,self.environment.clone())?;
            browser.call(&["open","about:blank"])?;let runtime=browser.evaluate("navigator.userAgent")?;let runtime=runtime.as_str().ok_or("Browser user agent missing")?;let version=pattern.captures(runtime).ok_or("Browser version missing")?[1].to_owned();self.report["chromium"]=json!(version);
            browser.call(&["network","har","start"])?;browser.call(&["open",&format!("{}/subs?api={}",app.origin,crate::fixture::quote(&format!("{}/backend",app.origin)))])?;browser.wait("localStorage.getItem('envCache') !== null")?;browser.click_visible(".empty-state-wrapper .nut-button")?;browser.popup(".add-sub-popup")?;
            let began=Instant::now();browser.call(&["upload","input[type=\"file\"]",input_file.to_str().ok_or("Invalid input path")?])?;let until=Instant::now();let items=loop{let items=transport::api(&app.origin,"subs","GET",None)?;if items.as_array().is_some_and(|items|!items.is_empty()){break items;}check(until.elapsed()<Duration::from_secs(20),"import_fields")?;std::thread::sleep(Duration::from_millis(50));};let wall=began.elapsed().as_secs_f64();check(items.as_array().is_some_and(|items|items.len()==1),"import_fields")?;let stored=&items[0];let display=stored["displayName"].as_str().ok_or("Imported display name missing")?;
            browser.call(&["wait","--text",display])?;browser.call(&["reload"])?;browser.call(&["wait","--text",display])?;let name=stored["name"].as_str().ok_or("Imported name missing")?;browser.call(&["open",&format!("{}/edit/subs/{}",app.origin,crate::fixture::quote(name))])?;
            let editor=format!("Array.from(document.querySelectorAll('input')).some(el => el.value === {})",serde_json::to_string(name)?);browser.wait(&editor)?;check(browser.evaluate(&editor)?==true,"import_fields")?;
            let har=directory.join("import.har");browser.call(&["network","har","stop",har.to_str().ok_or("Invalid private HAR path")?])?;let har=read(&har)?;let posts=har["log"]["entries"].as_array().ok_or("Import HAR entries missing")?.iter().filter(|entry|entry["request"]["method"]=="POST"&&entry["request"]["url"].as_str().and_then(|url|reqwest::Url::parse(url).ok()).is_some_and(|url|url.path()=="/backend/api/subs")).collect::<Vec<_>>();check(posts.len()==1&&posts[0]["response"]["status"]==201,"import_network")?;
            let payload:Value=serde_json::from_str(posts[0]["request"]["postData"]["text"].as_str().ok_or("Import request body missing")?)?;import_transform(original,&payload)?;check(payload.as_object().ok_or("Import object missing")?.iter().all(|(key,value)|stored.get(key)==Some(value)),"import_fields")?;check(read(app.state.join("sub-store.json"))?["subs"]==items,"persistence")?;
            let mut row=json!({"kind":kind,"case":"import","block":block,"startup_s":app.startup,"ui_import_wall_s":wall,"api_import_s":posts[0]["time"].as_f64().ok_or("HAR duration missing")?/1000.0,"payload_sha256":hash(&payload),"processors":stored["process"].as_array().ok_or("Imported processors missing")?.len(),"fields":payload.as_object().unwrap().len(),"reload":true,"editor":true,"restart":false});browser.close()?;let state=app.state.clone();drop(app);
            let previous=self.pristine.insert(kind.to_owned(),state);let directory=self.directory(&format!("restart-check-{kind}"))?;let restarted=self.server(kind,directory,true)?;check(transport::api(&restarted.origin,"subs","GET",None)?==items,"persistence")?;drop(restarted);row["restart"]=json!(true);if let Some(previous)=previous{self.pristine.insert(kind.to_owned(),previous);}self.report["imports"].as_array_mut().ok_or("Import report missing")?.push(row);self.publish()?;
        }}Ok(())
    }
    pub fn source_texts(&self,origin:&str,config:&Value)->Result<Value>{Ok(transport::api(origin,"preview/sub","POST",Some(&inspection(config)))?["processed"].clone())}
    pub fn capture(&mut self,original:&Value)->Result{
        let directory=self.directory("capture")?;let app=self.server("node",directory,true)?;let config=self.configure(&app.origin,original,false,"full")?;let began=Instant::now();let sources=self.source_texts(&app.origin,&config)?;let elapsed=began.elapsed().as_secs_f64();let sources=sources.as_array().ok_or("Captured source array missing")?;
        check(sources.len()==23&&sources.iter().all(Value::is_string)&&sources[22]==original["content"]&&sources[..22].iter().any(|source|source.as_str().is_some_and(|text|!text.is_empty())),"capture")?;
        let mut rows=Vec::new();for(index,source)in sources[..22].iter().enumerate(){let raw=source.as_str().ok_or("Captured source is not text")?.as_bytes();std::fs::write(self.args.scratch.join(format!("mirrors/{index}.txt")),raw)?;rows.push(json!({"index":index,"bytes":raw.len(),"sha256":digest(raw),"status":if raw.is_empty(){"empty_or_failed"}else{"nonempty"}}));}
        let nonempty=rows.iter().filter(|row|row["bytes"].as_u64().unwrap()>0).count();self.report["capture"]=json!({"elapsed_s":elapsed,"source_count":22,"before_preprocessing":true,"nonempty":nonempty,"empty_or_failed":22-nonempty,"sources":rows});drop(app);self.publish()
    }
    pub fn replay(&mut self,original:&Value)->Result{
        let mut expected=(0..22).map(|index|Ok(json!(std::fs::read_to_string(self.args.scratch.join(format!("mirrors/{index}.txt")))?))).collect::<Result<Vec<_>>>()?;expected.push(original["content"].clone());let expected=json!(expected);self.report["replay"]=json!([]);
        for kind in KINDS{let directory=self.directory(&format!("replay-{kind}"))?;let app=self.server(kind,directory,true)?;let config=self.configure(&app.origin,original,true,"full")?;let actual=self.source_texts(&app.origin,&config)?;let timeout_logs=std::fs::read_to_string(&app.owned.process.stdout)?.matches("exceeds the timeout").count()+std::fs::read_to_string(&app.owned.process.stderr)?.matches("exceeds the timeout").count();let row=replay_row(kind,&expected,&actual,timeout_logs)?;self.report["replay"].as_array_mut().unwrap().push(row);}
        self.publish()?;check(self.report["replay"].as_array().unwrap().iter().all(|row|row["equal"]==true),"capture")
    }
    pub fn counts(&self,reset:bool,real:bool)->Result<Value>{Ok(serde_json::from_slice(&transport::http(&format!("{}{}",self.fixture_origin,if reset{"/reset"}else{"/counts"}),if reset{"POST"}else{"GET"},reset.then_some(&json!({"real":real})))?)?)}
    pub fn fixture(&mut self,binary:&Path,meta_port:u16)->Result{
        let port=server::free_port()?;let directory=self.directory("fixture")?;let mut command=Command::new(binary);command.arg(self.args.scratch.join("mirrors")).arg(port.to_string()).arg(meta_port.to_string()).current_dir(&self.args.scratch).env_clear().envs(&self.environment);let mut owned=Owned::spawn(&mut command,&directory,"fixture")?;self.fixture_origin=format!("http://127.0.0.1:{port}");ready(&mut owned,&format!("{}/counts",self.fixture_origin))?;self.fixture=Some(owned);
        let began=Instant::now();let before=resources::counters(std::process::id())?.cpu;let fixture_before=resources::counters(self.fixture.as_ref().unwrap().pid())?.cpu;
        std::thread::scope(|scope|->Result{let handles=(0..8).map(|_|scope.spawn(||->Result{for _ in 0..20{transport::http(&format!("{}/health.js",self.fixture_origin),"GET",None)?;}Ok(())})).collect::<Vec<_>>();for handle in handles{handle.join().map_err(|_|"Fixture calibration thread panicked")??;}Ok(())})?;
        self.report["fixture_calibration"]=json!({"requests":160,"elapsed_s":began.elapsed().as_secs_f64(),"driver_cpu_s":resources::counters(std::process::id())?.cpu-before,"fixture_cpu_s":resources::counters(self.fixture.as_ref().unwrap().pid())?.cpu-fixture_before});Ok(())
    }
    pub fn provenance(&mut self)->Result<Value>{
        let mut sources=vec![(self.args.original.clone(),ORIGINAL),(self.args.rust.clone(),RUST),(self.args.frontend.clone(),FRONTEND)];if let Some(baseline)=&self.args.baseline{sources.push((baseline.clone(),RUST_BASELINE));}
        for(path,revision)in sources{check(self.command(Command::new("git").args(["rev-parse","HEAD"]).current_dir(&path),"source-revision",30)?.trim()==revision,"source")?;self.command(Command::new("git").args(["diff","--exit-code","HEAD"]).current_dir(path),"source-clean",30)?;}
        let rust_binary=digest(std::fs::read(self.args.rust.join("rust/target/release/sub-store-rust"))?);let baseline_binary=self.args.baseline.as_ref().map(|root|->Result<String>{Ok(digest(std::fs::read(root.join("rust/target/release/sub-store-rust"))?))}).transpose()?;
        check(baseline_binary.as_ref().is_none_or(|baseline|baseline!=&rust_binary)&&self.command(Command::new(&self.node).arg("--version"),"node-version",30)?.trim()=="v24.16.0","source")?;
        let harness=self.command(Command::new("git").args(["rev-parse","HEAD"]).current_dir(crate::root()),"harness-revision",30)?;
        let assets=crate::distribution::asset_hashes(&self.args.frontend.join("dist"))?.into_values().map(Value::String).collect::<Vec<_>>();
        Ok(json!({"original":ORIGINAL,"rust":RUST,"frontend":FRONTEND,"rust_baseline":self.args.baseline.as_ref().map(|_|RUST_BASELINE),"rust_baseline_binary_sha256":baseline_binary,"original_lock_sha256":digest(std::fs::read(self.args.original.join("backend/pnpm-lock.yaml"))?),"rust_lock_sha256":digest(std::fs::read(self.args.rust.join("rust/Cargo.lock"))?),"node_bundle_sha256":digest(std::fs::read(self.args.original.join("backend/dist/sub-store.bundle.js"))?),"rust_binary_sha256":rust_binary,"frontend_assets_sha256":hash(&json!(assets)),"node_version":"24.16.0","rust_toolchain":"1.98.0","browser_driver_version":"0.27.3","nextest_version":"0.9.149","harness_sha":harness.trim(),"harness_lock_sha256":digest(std::fs::read(crate::root().join("tests/Cargo.lock"))?),"runner":resources::runner()?}))
    }
    pub fn script(&mut self)->Result<String>{
        let token=std::env::var("BENCHMARK_ASSET_TOKEN").unwrap_or_default();let client=reqwest::blocking::Client::builder().timeout(Duration::from_secs(60)).user_agent("sub-store-nextest-performance").build()?;
        let bytes=fetch_pinned(||{let mut request=client.get(format!("https://api.github.com/repos/DSLZL/scripts/git/blobs/{SCRIPT_BLOB}"));if !token.is_empty(){request=request.bearer_auth(&token);}let reply:Value=serde_json::from_slice(&request.send()?.error_for_status()?.bytes()?)?;Ok(STANDARD.decode(reply["content"].as_str().ok_or("Pinned script body missing")?.split_whitespace().collect::<String>())?)},SCRIPT_HASH,||std::thread::sleep(Duration::from_secs(1)))?;Ok(String::from_utf8(bytes)?)
    }
    pub fn patch(&mut self,before:&str,after:&str,name:&str)->Result{
        let directory=self.directory("script-patch")?;std::fs::write(directory.join("pinned.js"),before)?;std::fs::write(directory.join("variant.js"),after)?;let result=process::run(Command::new("git").args(["diff","--no-index","--","pinned.js","variant.js"]).current_dir(&directory),&directory,"diff",Duration::from_secs(30))?;check(!result.timed_out&&(result.status.success()||result.status.code()==Some(1)),"script_anchor")?;std::fs::write(self.args.publish.join(name),result.stdout)?;Ok(())
    }
}
pub fn replay_row(kind:&str,expected:&Value,actual:&Value,timeouts:usize)->Result<Value>{
    let sources=actual.as_array().ok_or("Replay source array missing")?;check(sources.iter().all(Value::is_string),"capture")?;
    Ok(json!({"kind":kind,"expected_sha256":hash(expected),"actual_sha256":hash(actual),"equal":actual==expected,"source_count":sources.len(),"matching_sources":expected.as_array().ok_or("Expected replay missing")?.iter().zip(sources).filter(|(a,b)|a==b).count(),"timeout_log_entries":timeouts,"sources":sources.iter().enumerate().map(|(index,text)|{let bytes=text.as_str().unwrap().as_bytes();json!({"index":index,"bytes":bytes.len(),"sha256":digest(bytes)})}).collect::<Vec<_>>()}))
}
pub fn fetch_pinned(mut fetch:impl FnMut()->Result<Vec<u8>>,expected:&str,mut delay:impl FnMut())->Result<Vec<u8>>{
    for attempt in 0..3{match fetch(){Ok(bytes)=>{check(digest(&bytes)==expected,"asset")?;return Ok(bytes);},Err(_)if attempt<2=>delay(),Err(_)=>return Err(Box::new(Failure{code:"asset",observations:None}))}}Err("Unreachable asset retry".into())
}
pub fn now_ms()->f64{SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).expect("UTC clock after epoch").as_secs_f64()*1000.0}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn script_fetch_retries_io_but_never_accepts_a_changed_pinned_digest()->Result{
        let body=b"pinned script".to_vec();let mut attempts=0;let mut delays=0;let actual=fetch_pinned(||{attempts+=1;if attempts==1{Err(std::io::Error::from(std::io::ErrorKind::ConnectionReset).into())}else{Ok(body.clone())}},&digest(&body),||delays+=1)?;assert_eq!(actual,body);assert_eq!((attempts,delays),(2,1));assert!(fetch_pinned(||Ok(body.clone()),SCRIPT_HASH,||{}).is_err());Ok(())
    }
    #[test]
    fn ordered_raw_replay_retains_empty_sources_and_never_publishes_private_text()->Result{
        let mut sources=(0..22).map(|index|json!(if index==3{String::new()}else{format!("private-source-{index}\r\n节点")})).collect::<Vec<_>>();sources.push(json!("private-local-content"));let expected=json!(sources);
        for kind in KINDS{let mut changed=expected.clone();changed[1]=json!("changed");let mut reordered=expected.clone();reordered.as_array_mut().unwrap().swap(0,1);let mut missing=expected.clone();missing.as_array_mut().unwrap().pop();for actual in[expected.clone(),changed,reordered,missing]{let row=replay_row(kind,&expected,&actual,1)?;assert_eq!(row["equal"],actual==expected);public(&row,&BTreeSet::new())?;assert!(!serde_json::to_string(&row)?.contains("private-"));}}
        Ok(())
    }
}
