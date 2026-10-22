use super::*;
use std::{sync::{Arc,atomic::{AtomicBool,Ordering}},thread::{self,JoinHandle},time::{Duration,Instant}};

#[derive(Clone,Copy)]
pub struct Counters{pub cpu:f64,pub waited_cpu:f64,pub rss:u64,pub peak:u64,pub read:u64,pub written:u64,pub identity:u64}
#[cfg(target_os="linux")]
pub fn counters(pid:u32)->Result<Counters>{
    let base=std::path::PathBuf::from(format!("/proc/{pid}"));let stat=std::fs::read_to_string(base.join("stat"))?;let fields=stat.rsplit_once(") ").ok_or("Invalid process stat")?.1.split_whitespace().collect::<Vec<_>>();check(fields.len()>21,"resource")?;
    let number=|index:usize|->Result<u64>{Ok(fields[index].parse()?)};let ticks=unsafe{libc::sysconf(libc::_SC_CLK_TCK)};let pages=unsafe{libc::sysconf(libc::_SC_PAGESIZE)};check(ticks>0&&pages>0,"resource")?;
    let status=std::fs::read_to_string(base.join("status"))?;let peak=status.lines().find_map(|line|line.strip_prefix("VmHWM:")).and_then(|value|value.split_whitespace().next()).map(str::parse::<u64>).transpose()?.map_or(0,|value|value*1024);
    let io=std::fs::read_to_string(base.join("io"))?;let io_number=|name:&str|->Result<u64>{Ok(io.lines().find_map(|line|line.strip_prefix(name)).ok_or("Missing process IO counter")?.trim().parse()?)};
    Ok(Counters{cpu:(number(11)?+number(12)?)as f64/ticks as f64,waited_cpu:(number(13)?+number(14)?)as f64/ticks as f64,rss:number(21)?*pages as u64,peak,read:io_number("read_bytes:")?,written:io_number("write_bytes:")?,identity:number(19)?})
}
#[cfg(not(target_os="linux"))]
pub fn counters(_pid:u32)->Result<Counters>{Err("Performance resource accounting requires Linux".into())}

#[cfg(target_os="linux")]
fn descendants(pid:u32)->Result<Vec<u32>>{
    let path=format!("/proc/{pid}/task/{pid}/children");let children=match std::fs::read_to_string(path){Ok(text)=>text,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>return Ok(Vec::new()),Err(error)=>return Err(error.into())};let mut found=Vec::new();
    for pid in children.split_whitespace(){let pid=pid.parse::<u32>()?;found.push(pid);found.extend(descendants(pid)?);}Ok(found)
}
#[cfg(not(target_os="linux"))]
fn descendants(_pid:u32)->Result<Vec<u32>>{Err("Linux child accounting required".into())}
fn auxiliary(pid:Option<u32>)->Result<(f64,u64)>{
    let Some(pid)=pid else{return Ok((0.0,0));};let parent=counters(pid)?;let mut cpu=parent.cpu+parent.waited_cpu;let mut rss=parent.rss;
    for child in descendants(pid)?{match counters(child){Ok(child)=>{cpu+=child.cpu;rss+=child.rss;},Err(error)if error.downcast_ref::<std::io::Error>().is_some_and(|error|error.kind()==std::io::ErrorKind::NotFound)=>{},Err(error)=>return Err(error)}}Ok((cpu,rss))
}
#[cfg(target_os="linux")]
fn thread_cpu()->Result<f64>{let mut time=std::mem::MaybeUninit::<libc::timespec>::uninit();check(unsafe{libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID,time.as_mut_ptr())}==0,"resource")?;let time=unsafe{time.assume_init()};Ok(time.tv_sec as f64+time.tv_nsec as f64/1e9)}
#[cfg(not(target_os="linux"))]
fn thread_cpu()->Result<f64>{Err("Linux sampler clock required".into())}

type Samples=Result<(Vec<(f64,u64)>,u64,f64)>;
pub struct Meter{
    pid:u32,fixture:Option<u32>,auxiliary:Option<u32>,before:Counters,driver:f64,fixture_cpu:f64,aux_cpu:f64,started:Instant,
    stop:Arc<AtomicBool>,sampler:Option<JoinHandle<Samples>>,
}
impl Meter{
    pub fn start(pid:u32,fixture:Option<u32>,aux:Option<u32>)->Result<Self>{
        check(cfg!(target_os="linux"),"resource")?;let before=counters(pid)?;let driver=counters(std::process::id())?.cpu;let fixture_cpu=fixture.map(counters).transpose()?.map_or(0.0,|value|value.cpu);let aux_cpu=auxiliary(aux)?.0;
        let started=Instant::now();let stop=Arc::new(AtomicBool::new(false));let flag=stop.clone();
        let sampler=thread::spawn(move||->Samples{let cpu_start=thread_cpu()?;let mut samples=Vec::new();let mut peak=0;while !flag.load(Ordering::Relaxed){let snapshot=counters(pid)?;check(snapshot.identity==before.identity,"resource")?;samples.push((started.elapsed().as_secs_f64(),snapshot.rss));peak=peak.max(auxiliary(aux)?.1);thread::sleep(Duration::from_millis(50));}Ok((samples,peak,thread_cpu()?-cpu_start))});
        Ok(Self{pid,fixture,auxiliary:aux,before,driver,fixture_cpu,aux_cpu,started,stop,sampler:Some(sampler)})
    }
    pub fn finish(mut self,completed:usize)->Result<Value>{
        let elapsed=self.started.elapsed().as_secs_f64();let after=counters(self.pid)?;check(after.identity==self.before.identity,"resource")?;let driver=counters(std::process::id())?.cpu;let fixture=self.fixture.map(counters).transpose()?;let aux=auxiliary(self.auxiliary)?.0;self.stop.store(true,Ordering::Relaxed);
        let(trace,aux_peak,sampler_cpu)=self.sampler.take().ok_or("Sampler missing")?.join().map_err(|_|"Sampler panicked")??;
        let end_rss=after.rss;thread::sleep(Duration::from_millis(250));let settled=counters(self.pid)?;check(settled.identity==self.before.identity&&after.read>=self.before.read&&after.written>=self.before.written,"resource")?;
        let intervals=trace.windows(2).map(|pair|pair[1].0-pair[0].0).collect::<Vec<_>>();let cpu=after.cpu-self.before.cpu;
        Ok(json!({"elapsed_s":elapsed,"cpu_s":cpu,"cpu_s_per_request":if completed>0{Some(cpu/completed as f64)}else{None},"rps":completed as f64/elapsed,"completed":completed,"rss_peak_bytes":trace.iter().map(|(_,rss)|*rss).max().unwrap_or(0),"rss_end_bytes":end_rss,"rss_settled_bytes":settled.rss,"os_lifetime_peak_bytes":if after.peak>0{Some(after.peak)}else{None},"io_read_bytes":after.read-self.before.read,"io_write_bytes":after.written-self.before.written,"sampler_cpu_s":sampler_cpu,"driver_cpu_s":driver-self.driver,"fixture_cpu_s":fixture.map_or(0.0,|value|value.cpu-self.fixture_cpu),"fixture_rss_bytes":fixture.map_or(0,|value|value.rss),"detector_cpu_s":aux-self.aux_cpu,"detector_rss_peak_bytes":aux_peak,"sample_interval_s":distribution(&intervals)?,"rss_trace":trace}))
    }
}
impl Drop for Meter{fn drop(&mut self){self.stop.store(true,Ordering::Relaxed);if let Some(sampler)=self.sampler.take(){let _=sampler.join();}}}

pub fn cpu_quota(path:&Path)->Result<Value>{
    let text=match std::fs::read_to_string(path){Ok(text)=>text,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>return Ok(json!({"cpu_quota_us":null,"cpu_period_us":null})),Err(error)=>return Err(error.into())};let fields=text.split_whitespace().collect::<Vec<_>>();check(fields.len()==2,"resource")?;let quota=if fields[0]=="max"{None}else{Some(fields[0].parse::<u64>()?)};let period=fields[1].parse::<u64>()?;check(period>0,"resource")?;Ok(json!({"cpu_quota_us":quota,"cpu_period_us":period}))
}
#[cfg(target_os="linux")]
pub fn runner()->Result<Value>{
    let model=std::fs::read_to_string("/proc/cpuinfo")?.lines().find_map(|line|line.strip_prefix("model name").and_then(|line|line.strip_prefix('\t')).and_then(|line|line.trim_start().strip_prefix(':')).map(|line|line.trim().to_owned())).ok_or("CPU model missing")?;
    let mut set=std::mem::MaybeUninit::<libc::cpu_set_t>::zeroed();check(unsafe{libc::sched_getaffinity(0,std::mem::size_of::<libc::cpu_set_t>(),set.as_mut_ptr())}==0,"resource")?;let set=unsafe{set.assume_init()};let affinity=unsafe{libc::CPU_COUNT(&set)};
    let memory=std::fs::read_to_string("/proc/meminfo")?.lines().find_map(|line|line.strip_prefix("MemTotal:")).and_then(|value|value.split_whitespace().next()).ok_or("Memory total missing")?.parse::<u64>()?*1024;
    let limit=match std::fs::read_to_string("/sys/fs/cgroup/memory.max"){Ok(text)if text.trim()!="max"=>Some(text.trim().parse::<u64>()?),Ok(_)=>None,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>None,Err(error)=>return Err(error.into())};
    let load=std::fs::read_to_string("/proc/loadavg")?.split_whitespace().take(3).map(str::parse::<f64>).collect::<std::result::Result<Vec<_>,_>>()?;
    let logical=unsafe{libc::sysconf(libc::_SC_NPROCESSORS_ONLN)};check(logical>0&&affinity>0,"resource")?;let mut report=json!({"target":"linux_x64","cpu_model":model,"logical_cpus":logical,"affinity_cpus":affinity,"memory_bytes":memory,"memory_limit_bytes":limit,"load_before":load});report.as_object_mut().unwrap().extend(cpu_quota(Path::new("/sys/fs/cgroup/cpu.max"))?.as_object().unwrap().clone());Ok(report)
}
#[cfg(not(target_os="linux"))]
pub fn runner()->Result<Value>{Err("Performance provenance requires Linux".into())}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn missing_unlimited_and_bounded_cgroup_quotas_have_explicit_units()->Result{
        let root=crate::process::scratch("performance-cgroup")?;let path=root.join("cpu.max");assert_eq!(cpu_quota(&path)?,json!({"cpu_quota_us":null,"cpu_period_us":null}));std::fs::write(&path,"max 100000\n")?;assert_eq!(cpu_quota(&path)?,json!({"cpu_quota_us":null,"cpu_period_us":100000}));std::fs::write(&path,"200000 100000\n")?;assert_eq!(cpu_quota(&path)?,json!({"cpu_quota_us":200000,"cpu_period_us":100000}));Ok(())
    }
    #[test]
    #[ignore="owned child for the Linux resource-accounting self-check"]
    fn meter_child()->Result{
        crate::actions_only();let root=std::path::PathBuf::from(std::env::var_os("PERF_METER_STATE").ok_or("Missing meter child state")?);std::fs::write(root.join("ready"),b"ready")?;
        let wait=|name:&str|->Result{let started=Instant::now();while !root.join(name).is_file(){check(started.elapsed()<Duration::from_secs(10),"timeout")?;thread::sleep(Duration::from_millis(5));}Ok(())};wait("go")?;let started=Instant::now();let mut count=0_u64;while started.elapsed()<Duration::from_millis(250){count=std::hint::black_box(count.wrapping_add(1));}std::fs::write(root.join("done"),count.to_string())?;wait("stop")
    }
    #[cfg(target_os="linux")]
    #[test]
    fn resource_units_measure_only_the_owned_server_and_retain_sampler_intervals()->Result{
        let root=crate::process::scratch("performance-meter")?;let mut child=crate::process::Running::spawn(std::process::Command::new(std::env::current_exe()?).args(["--exact","performance::resources::tests::meter_child","--ignored","--nocapture"]).env("PERF_METER_STATE",&root),&root,"child")?;
        let wait=|name:&str|->Result{let started=Instant::now();while !root.join(name).is_file(){check(started.elapsed()<Duration::from_secs(10),"timeout")?;thread::sleep(Duration::from_millis(5));}Ok(())};wait("ready")?;let meter=Meter::start(child.child.id(),None,None)?;std::fs::write(root.join("go"),b"go")?;wait("done")?;let result=meter.finish(1)?;
        check(result["cpu_s"].as_f64().unwrap()>0.1&&result["rss_peak_bytes"].as_u64().unwrap()>1048576&&result["os_lifetime_peak_bytes"].as_u64().unwrap()>=result["rss_peak_bytes"].as_u64().unwrap()&&result["rss_trace"].as_array().unwrap().len()>1,"resource")?;
        std::fs::write(root.join("stop"),b"stop")?;check(child.wait(Duration::from_secs(5))?.0.success(),"resource")?;public(&result,&BTreeSet::new())
    }
}
