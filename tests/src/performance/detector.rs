use super::*;
use std::{path::PathBuf,sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}},time::Duration};
use tokio::{io::{AsyncBufReadExt,AsyncReadExt,AsyncWriteExt,BufReader},net::{TcpListener,TcpSocket,TcpStream},task::JoinHandle};

struct Batch(Vec<JoinHandle<()>>);
impl Drop for Batch{fn drop(&mut self){for task in &self.0{task.abort();}}}
struct State{counts:BTreeMap<&'static str,u64>,batches:BTreeMap<u64,Batch>,next_pid:u64,real:bool}
impl State{fn reset(&mut self){self.counts=["sources","scripts","starts","stops","selected","probes","unique_probes","active","max_active"].into_iter().map(|key|(key,0)).collect();}}
#[derive(Clone)]
struct Detector{directory:PathBuf,meta_port:u16,state:Arc<Mutex<State>>,client:reqwest::Client}
async fn listener(port:u16)->Result<TcpListener>{let socket=TcpSocket::new_v4()?;socket.bind(std::net::SocketAddr::from(([127,0,0,1],port)))?;Ok(socket.listen(4096)?)}
impl Detector{
    fn new(directory:PathBuf,meta_port:u16)->Result<Self>{let mut state=State{counts:BTreeMap::new(),batches:BTreeMap::new(),next_pid:1,real:false};state.reset();Ok(Self{directory,meta_port,state:Arc::new(Mutex::new(state)),client:reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(60)).build()?})}
    async fn response(&self,method:&str,path:&str,body:&[u8],identity:Option<&AtomicBool>)->Result<(u16,Vec<u8>)>{
        if let Some(identity)=identity{
            {let mut state=self.state.lock().map_err(|_|"Detector lock poisoned")?;*state.counts.get_mut("active").unwrap()+=1;let active=state.counts["active"];let maximum=state.counts["max_active"].max(active);state.counts.insert("max_active",maximum);*state.counts.get_mut("probes").unwrap()+=1;if !identity.swap(true,Ordering::Relaxed){*state.counts.get_mut("unique_probes").unwrap()+=1;}}
            tokio::time::sleep(Duration::from_millis(1)).await;*self.state.lock().map_err(|_|"Detector lock poisoned")?.counts.get_mut("active").unwrap()-=1;return Ok((204,vec![]));
        }
        match path{
            "/counts"=>Ok((200,canonical(&json!(self.state.lock().map_err(|_|"Detector lock poisoned")?.counts)))),
            "/reset"=>{let data:Value=serde_json::from_slice(if body.is_empty(){b"{}"}else{body})?;let mut state=self.state.lock().map_err(|_|"Detector lock poisoned")?;check(state.batches.is_empty(),"fixture")?;state.reset();state.real=data["real"].as_bool().unwrap_or(false);Ok((200,b"{}".to_vec()))},
            "/health.js"=>{*self.state.lock().map_err(|_|"Detector lock poisoned")?.counts.get_mut("scripts").unwrap()+=1;Ok((200,std::fs::read(self.directory.join("stress.js"))?))},
            "/start"=>{
                check(method=="POST","fixture")?;let data:Value=serde_json::from_slice(body)?;let proxies=data["proxies"].as_array().ok_or("Detector proxies missing")?;
                let(real,pid)={let mut state=self.state.lock().map_err(|_|"Detector lock poisoned")?;*state.counts.get_mut("starts").unwrap()+=1;*state.counts.get_mut("selected").unwrap()+=proxies.len()as u64;let pid=state.next_pid;state.next_pid+=1;(state.real,pid)};
                if real{return self.forward(path,body).await;}
                let mut batch=Batch(Vec::new());let mut ports=Vec::new();for _ in proxies{let listener=listener(0).await?;ports.push(listener.local_addr()?.port());let detector=self.clone();let marker=Arc::new(AtomicBool::new(false));batch.0.push(tokio::spawn(async move{while let Ok((socket,_))=listener.accept().await{let detector=detector.clone();let marker=marker.clone();tokio::spawn(async move{detector.handle(socket,Some(marker)).await;});}}));}
                self.state.lock().map_err(|_|"Detector lock poisoned")?.batches.insert(pid,batch);Ok((200,canonical(&json!({"pid":pid,"ports":ports}))))
            },
            "/stop"=>{
                check(method=="POST","fixture")?;let real={let mut state=self.state.lock().map_err(|_|"Detector lock poisoned")?;*state.counts.get_mut("stops").unwrap()+=1;state.real};if real{return self.forward(path,body).await;}
                let data:Value=serde_json::from_slice(body)?;let mut state=self.state.lock().map_err(|_|"Detector lock poisoned")?;for pid in data["pid"].as_array().ok_or("Detector PID list missing")?{check(state.batches.remove(&pid.as_u64().ok_or("Invalid detector PID")?).is_some(),"fixture")?;}Ok((200,b"{}".to_vec()))
            },
            _ if path.starts_with("/source/")=>{let index=path.strip_prefix("/source/").ok_or("Source index missing")?.parse::<usize>()?;check(index<22,"fixture")?;*self.state.lock().map_err(|_|"Detector lock poisoned")?.counts.get_mut("sources").unwrap()+=1;Ok((200,std::fs::read(self.directory.join(format!("{index}.txt")))?))},
            _=>Ok((404,vec![]))
        }
    }
    async fn forward(&self,path:&str,body:&[u8])->Result<(u16,Vec<u8>)>{let response=self.client.post(format!("http://127.0.0.1:{}{path}",self.meta_port)).header("Content-Type","application/json").body(body.to_vec()).send().await?;check(response.status().is_success(),"fixture")?;Ok((200,response.bytes().await?.to_vec()))}
    async fn handle(&self,socket:TcpStream,identity:Option<Arc<AtomicBool>>){
        let(mut reader,mut writer)=socket.into_split();let result=async{
            let mut reader=BufReader::new(&mut reader);let mut line=Vec::new();let read=tokio::time::timeout(Duration::from_secs(30),reader.read_until(b'\n',&mut line)).await??;check(read>0&&line.len()<1048576,"fixture")?;
            let first=std::str::from_utf8(&line)?;let fields=first.split_whitespace().collect::<Vec<_>>();check(fields.len()==3,"fixture")?;let method=fields[0].to_owned();let path=fields[1].to_owned();let mut length=0_usize;let mut total=line.len();
            loop{line.clear();check(tokio::time::timeout(Duration::from_secs(30),reader.read_until(b'\n',&mut line)).await??>0,"fixture")?;total+=line.len();check(total<=1048576,"fixture")?;if line==b"\r\n"{break;}let header=line.iter().map(|byte|char::from(*byte)).collect::<String>();if let Some((key,value))=header.split_once(':')&&key.eq_ignore_ascii_case("content-length"){length=value.trim().parse()?;}}
            check(length<=128*1024*1024,"fixture")?;let mut body=vec![0;length];tokio::time::timeout(Duration::from_secs(30),reader.read_exact(&mut body)).await??;self.response(&method,&path,&body,identity.as_deref()).await
        }.await;
        let(status,body)=result.unwrap_or((500,vec![]));let header=format!("HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",body.len());let _=writer.write_all(header.as_bytes()).await;let _=writer.write_all(&body).await;let _=writer.shutdown().await;
    }
    async fn serve(self,listener:TcpListener)->Result{loop{let(socket,_)=listener.accept().await?;let detector=self.clone();tokio::spawn(async move{detector.handle(socket,None).await;});}}
}
pub fn run(directory:PathBuf,port:u16,meta_port:u16)->Result{
    crate::actions_only();#[cfg(target_os="linux")]{let mut limits=std::mem::MaybeUninit::<libc::rlimit>::uninit();check(unsafe{libc::getrlimit(libc::RLIMIT_NOFILE,limits.as_mut_ptr())}==0,"resource")?;let mut limits=unsafe{limits.assume_init()};limits.rlim_cur=limits.rlim_max;check(unsafe{libc::setrlimit(libc::RLIMIT_NOFILE,&limits)}==0,"resource")?;}
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async{let detector=Detector::new(directory,meta_port)?;detector.serve(listener(port).await?).await})
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn distinct_ports_count_every_identity_retry_and_cleanup_without_exposing_errors()->Result{
        let root=crate::process::scratch("performance-detector")?;std::fs::write(root.join("stress.js"),b"fixture script")?;let(ready,receive)=std::sync::mpsc::channel();let(stop,stopped)=tokio::sync::oneshot::channel::<()>();
        let thread=std::thread::spawn(move||->Result{tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async{let listener=listener(0).await?;ready.send(listener.local_addr()?.port())?;let detector=Detector::new(root,0)?;tokio::select!{result=detector.serve(listener)=>result,_=stopped=>Ok(())}})});
        let origin=format!("http://127.0.0.1:{}",receive.recv_timeout(Duration::from_secs(10))?);let started:Value=serde_json::from_slice(&transport::http(&format!("{origin}/start"),"POST",Some(&json!({"proxies":[{},{},{}]})))?)?;let ports=started["ports"].as_array().unwrap();assert_eq!(ports.iter().filter_map(Value::as_u64).collect::<BTreeSet<_>>().len(),3);
        std::thread::scope(|scope|->Result{let handles=ports.iter().map(|port|scope.spawn(move||transport::http(&format!("http://127.0.0.1:{port}/"),"HEAD",None))).collect::<Vec<_>>();for handle in handles{handle.join().map_err(|_|"Probe self-check panicked")??;}Ok(())})?;
        transport::http(&format!("http://127.0.0.1:{}/",ports[0]),"HEAD",None)?;transport::http(&format!("{origin}/stop"),"POST",Some(&json!({"pid":[started["pid"]]})))?;let counts:Value=serde_json::from_slice(&transport::http(&format!("{origin}/counts"),"GET",None)?)?;assert_eq!(counts["unique_probes"],3);assert_eq!(counts["probes"],4);assert_eq!(counts["active"],0);transport::http(&format!("{origin}/reset"),"POST",Some(&json!({})))?;
        let error=transport::http(&format!("{origin}/missing"),"GET",None).unwrap_err();let report=failure(error.as_ref());assert_eq!(report["failure_observations"]["http_status"],404);public(&report,&BTreeSet::new())?;let _=stop.send(());thread.join().map_err(|_|"Detector self-check panicked")??;Ok(())
    }
}
