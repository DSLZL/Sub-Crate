use crate::{Result,process,require,server::Environment};
use std::{path::{Path,PathBuf},process::Command,time::{Duration,SystemTime}};

pub enum Target { Native(PathBuf), Docker(String) }
impl Target {
    pub fn from_env()->Result<Self>{
        if let Ok(image)=std::env::var("TEST_DOCKER_PROBE_IMAGE"){require(!image.is_empty(),"Empty probe image")?;return Ok(Self::Docker(image));}
        Ok(Self::Native(PathBuf::from(std::env::var_os("TEST_PROBE_BINARY").ok_or("Missing actual script probe")?).canonicalize()?))
    }
    pub fn command(&self,environment:&Environment)->Result<Command>{
        let image=match self{Self::Native(binary)=>return Ok(Command::new(binary)),Self::Docker(image)=>image};
        let get=|key:&str|environment.get(std::ffi::OsStr::new(key)).and_then(|value|value.to_str()).ok_or_else(||format!("Missing container probe environment {key}"));
        let state=Path::new(get("SUB_STORE_COMPAT_STATE_DIR")?).canonicalize()?;
        let nonce=SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_nanos();let name=format!("nextest-probe-{}-{nonce}",std::process::id());
        let user=crate::server::docker_user()?;
        let mut command=Command::new("docker");command.args(["run","--name",&name,"--network","host","--read-only","--user",&user,"--cap-drop=ALL","--security-opt=no-new-privileges","--mount",&format!("type=bind,source={},target=/state",state.display()),"--env","SUB_STORE_COMPAT_STATE_DIR=/state","--env","SUB_STORE_DATA_BASE_PATH=/state"]);
        for(key,value)in environment{
            let key=key.to_str().ok_or("Invalid probe environment key")?;let value=value.to_str().ok_or("Invalid probe environment value")?;
            if ["SUB_STORE_COMPAT_STATE_DIR","SUB_STORE_DATA_BASE_PATH"].contains(&key){continue;}
            if key.starts_with("SUB_STORE_")||key=="TZ"{
                let value=if ["SUB_STORE_MMDB_COUNTRY_PATH","SUB_STORE_MMDB_ASN_PATH"].contains(&key){let relative=Path::new(value).strip_prefix(&state)?;format!("/state/{}",relative.to_string_lossy().replace('\\',"/"))}else{value.to_owned()};
                command.args(["--env",&format!("{key}={value}")]);
            }
        }
        command.arg(image);Ok(command)
    }
    pub fn run(&self,command:&mut Command,output:&Path,label:&str)->Result<process::Completed>{
        let container=if matches!(self,Self::Docker(_)){let mut args=command.get_args();let name=args.find(|arg|*arg=="--name").and_then(|_|args.next()).and_then(|name|name.to_str()).ok_or("Owned probe container name missing")?;require(name.starts_with(&format!("nextest-probe-{}-",std::process::id())),"Refusing cleanup of an unowned probe")?;Some(name.to_owned())}else{None};
        let result=process::run_direct(command,output,label,Duration::from_secs(60),"Direct probe ownership preserves native watchdog status; container cleanup is scoped to the owned name");
        if let Some(name)=container{process::checked(Command::new("docker").args(["rm","--force",&name]),output,&format!("{label}-container-cleanup"),Duration::from_secs(30))?;}
        result
    }
}
