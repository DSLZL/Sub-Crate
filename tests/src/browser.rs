use crate::{Result,json,process,require,server::Environment};
use base64::Engine;
use serde_json::Value;
use std::{path::{Path,PathBuf},process::Command,time::Duration};

pub struct Browser {pub output:PathBuf,pub session:String,pub environment:Environment,pub executable:PathBuf,index:usize,closed:bool}
impl Browser {
    pub fn new(output:&Path,environment:Environment)->Result<Self> {
        let executable=PathBuf::from(std::env::var_os("TEST_BROWSER").ok_or("Missing installed agent-browser executable")?).canonicalize()?;
        let session=format!("nextest-{}-{}",std::process::id(),crate::service::clock(false));
        Ok(Self{output:output.to_owned(),session,environment,executable,index:0,closed:false})
    }
    pub fn call(&mut self,args:&[&str])->Result<Value> {
        self.index+=1;let label=format!("browser-{:04}-{}",self.index,args.first().ok_or("Missing browser command")?);
        let mut command=Command::new(&self.executable);command.args(["--session",&self.session,"--json"]).args(args).current_dir(crate::root());
        for (key,_) in std::env::vars_os().filter(|(key,_)|key.to_string_lossy().starts_with("AGENT_BROWSER_")) {command.env_remove(key);}
        command.envs(&self.environment).env("AGENT_BROWSER_DEFAULT_TIMEOUT","15000");
        let result=if cfg!(windows) {process::run_direct(&mut command,&self.output,&label,Duration::from_secs(45),"File-backed native browser CLI avoids detached Windows daemon pipe inheritance")?} else {process::run(&mut command,&self.output,&label,Duration::from_secs(45))?};
        require(result.success(),format!("{label} failed: {}",result.stderr))?;let value=json::parse(&result.stdout)?;require(value["success"]==true,format!("{label}: {}",value["error"]))?;Ok(value["data"].clone())
    }
    pub fn evaluate(&mut self,script:&str)->Result<Value> {Ok(self.call(&["eval","-b",&base64::engine::general_purpose::STANDARD.encode(script)])?["result"].clone())}
    pub fn wait(&mut self,script:&str)->Result {self.call(&["wait","--fn",script])?;Ok(())}
    pub fn expect_value(&mut self,selector:&str,expected:&str)->Result {
        self.wait(&format!("document.querySelector({})?.value === {}",json::stringify(&serde_json::json!(selector)),json::stringify(&serde_json::json!(expected))))?;
        require(self.call(&["get","value",selector])?["value"]==expected,"Editor value did not survive reload/save")
    }
    pub fn popup(&mut self,selector:&str)->Result {
        self.wait(&format!(r#"(() => {{const popups=Array.from(document.querySelectorAll({})).filter(el=>el.getClientRects().length);if(popups.length!==1)return false;const popup=popups[0],r=popup.getBoundingClientRect();return r.width>0&&r.height>0&&r.top>=0&&r.left>=0&&r.bottom<=innerHeight&&r.right<=innerWidth&&popup.contains(document.elementFromPoint(r.left+r.width/2,r.top+r.height/2))&&popup.getAnimations({{subtree:true}}).every(a=>a.playState!=='running');}})()"#,json::stringify(&serde_json::json!(selector))))
    }
    pub fn click_visible(&mut self,selector:&str)->Result {
        self.call(&["wait",selector])?;let quoted=json::stringify(&serde_json::json!(selector));
        self.evaluate(&format!("(() => {{document.querySelector({quoted}).scrollIntoView({{block:'center',inline:'center',behavior:'instant'}});return true;}})()"))?;
        self.wait(&format!(r#"(async () => {{const el=document.querySelector({quoted}),r=el.getBoundingClientRect();await new Promise(requestAnimationFrame);await new Promise(requestAnimationFrame);const settled=el.getBoundingClientRect();let stopped=el.getAnimations({{subtree:true}}).every(a=>a.playState!=='running');for(let p=el.parentElement;p;p=p.parentElement)stopped=stopped&&p.getAnimations().every(a=>a.playState!=='running');return r.width>0&&r.height>0&&r.top>=0&&r.left>=0&&r.bottom<=innerHeight&&r.right<=innerWidth&&el.contains(document.elementFromPoint(r.left+r.width/2,r.top+r.height/2))&&!el.matches(':disabled,[aria-disabled="true"],.nut-button--loading,.nut-button--disabled')&&document.fonts.status==='loaded'&&stopped&&['x','y','width','height'].every(k=>Math.abs(r[k]-settled[k])<0.25);}})()"#))?;
        self.call(&["click",selector])?;Ok(())
    }
    pub fn click_name(&mut self,name:&str,role:Option<&str>)->Result {
        self.call(&["wait","--text",name])?;
        if let Some(role)=role {let snapshot=self.call(&["snapshot","-i"])?;let matches=snapshot["refs"].as_object().ok_or("Missing browser snapshot refs")?.iter().filter(|(_,value)|value["name"]==name&&value["role"]==role).map(|(key,_)|key).collect::<Vec<_>>();require(matches.len()==1,format!("Expected unique {role} {name}"))?;self.call(&["click",&format!("@{}",matches[0])])?;} else {self.call(&["find","text",name,"click","--exact"])?;}Ok(())
    }
    pub fn diagnostics(&mut self,label:&str) {for args in [vec!["snapshot","-i"],vec!["errors"],vec!["console"]] {let _=self.call(&args);}let path=self.output.join(format!("{label}.png"));let _=self.call(&["screenshot",&path.to_string_lossy()]);}
    pub fn close(&mut self)->Result {self.call(&["close"])?;self.closed=true;Ok(())}
}
impl Drop for Browser {fn drop(&mut self) {if self.index>0&&!self.closed {let _=self.close();}}}
