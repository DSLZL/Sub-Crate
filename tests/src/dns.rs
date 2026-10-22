use crate::{Result, require};
use base64::Engine;
use std::net::IpAddr;

pub fn name_bytes(domain: &str) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    for label in domain.split('.') {
        require(
            label.is_ascii() && !label.is_empty() && label.len() <= 63,
            "Invalid fixture DNS label",
        )?;
        result.push(label.len() as u8);
        result.extend_from_slice(label.as_bytes());
    }
    result.push(0);
    Ok(result)
}

fn words(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|word| word.to_be_bytes()).collect()
}

pub fn query_bytes(domain: &str, kind: u16, edns: Option<IpAddr>) -> Result<Vec<u8>> {
    let mut additional = Vec::new();
    if let Some(address) = edns {
        let (family, prefix, packed) = match address {
            IpAddr::V4(address) => (1, 24, address.octets().to_vec()),
            IpAddr::V6(address) => (2, 56, address.octets().to_vec()),
        };
        let mut subnet = words(&[family]);
        subnet.extend([prefix, 0]);
        subnet.extend_from_slice(&packed[..usize::from(prefix) / 8]);
        let mut option = words(&[8, subnet.len() as u16]);
        option.extend(subnet);
        additional.push(0);
        additional.extend(words(&[41, 4096, 0, 0, option.len() as u16]));
        additional.extend(option);
    }
    let mut packet = words(&[0, 0x100, 1, 0, 0, u16::from(!additional.is_empty())]);
    packet.extend(name_bytes(domain)?);
    packet.extend(words(&[kind, 1]));
    packet.extend(additional);
    Ok(packet)
}

pub fn query_path(mode: &str, domain: &str, kind: u16, edns: Option<IpAddr>) -> Result<String> {
    Ok(format!(
        "/host/dns/{mode}?dns={}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(query_bytes(domain, kind, edns)?)
    ))
}

pub fn response_bytes(packet: &[u8], mode: &str) -> Result<Vec<u8>> {
    require(packet.len() >= 12, "Truncated DNS header")?;
    let mut offset = 12;
    while let Some(length) = packet.get(offset).copied().filter(|length| *length != 0) {
        require(length <= 63, "Fixture accepts uncompressed labels only")?;
        offset += usize::from(length) + 1;
    }
    let end = offset + 5;
    require(end <= packet.len(), "Truncated DNS question")?;
    let kind = u16::from_be_bytes([packet[offset + 1], packet[offset + 2]]);
    require(
        packet[..10] == words(&[0, 0x100, 1, 0, 0]) && packet[offset + 3..end] == [0, 1],
        "Unexpected fixture DNS query header",
    )?;
    if mode == "malformed" {
        return Ok(vec![0, 1]);
    }
    let mut records = Vec::new();
    if !["empty", "serial-empty", "nxdomain"].contains(&mode) {
        records.push((5, name_bytes("alias.fixture")?));
        let answer_kind = if mode == "wrong" { 28 } else { kind };
        let addresses = if answer_kind == 28 {
            vec!["2001:db8::7"]
        } else {
            vec!["198.51.100.7", "198.51.100.9"]
        };
        for address in addresses {
            let data = match address.parse::<IpAddr>()? {
                IpAddr::V4(ip) => ip.octets().to_vec(),
                IpAddr::V6(ip) => ip.octets().to_vec(),
            };
            records.push((answer_kind, data));
        }
    }
    let flags = if ["nxdomain", "rcode"].contains(&mode) {
        0x8183
    } else {
        0x8180
    };
    let mut response = words(&[0, flags, 1, records.len() as u16, 0, 0]);
    response.extend_from_slice(&packet[12..end]);
    for (kind, data) in records {
        response.extend([0xc0, 0x0c]);
        response.extend(words(&[kind, 1, 0, 60, data.len() as u16]));
        response.extend(data);
    }
    Ok(response)
}

pub fn question_name(packet: &[u8]) -> Result<String> {
    let mut labels=Vec::new(); let mut offset=12;
    loop {
        let length=usize::from(*packet.get(offset).ok_or("Truncated DNS question")?); offset+=1;
        if length==0 { return Ok(labels.join(".")); }
        require(length<=63,"Compressed fixture question")?;
        let label=std::str::from_utf8(packet.get(offset..offset+length).ok_or("Truncated DNS label")?)?;
        require(label.is_ascii(),"Non-ASCII DNS fixture label")?;
        labels.push(label.to_owned()); offset+=length;
    }
}

pub fn tls_config() -> Result<std::sync::Arc<tokio_rustls::rustls::ServerConfig>> {
    use tokio_rustls::rustls::{ServerConfig,pki_types::{CertificateDer,PrivateKeyDer,pem::PemObject}};
    let fixtures=crate::compat().join("fixtures");
    let certificates=CertificateDer::pem_file_iter(fixtures.join("dns-cert.pem"))?.collect::<std::result::Result<Vec<_>,_>>()?;
    let private_key=PrivateKeyDer::from_pem_file(fixtures.join("dns-key.pem"))?;
    Ok(std::sync::Arc::new(ServerConfig::builder().with_no_client_auth().with_single_cert(certificates,private_key)?))
}

mod native {
    use super::*;
    use serde_json::{Value,json};
    use std::collections::BTreeMap;
    use std::io::{Read,Write};
    use std::net::{TcpListener,TcpStream,UdpSocket,Shutdown};
    use std::sync::{Arc,Mutex,Condvar,atomic::{AtomicBool,Ordering}};
    use std::thread::JoinHandle;
    use std::time::{Duration,Instant};
    use tokio_rustls::rustls::{ServerConnection,StreamOwned};

    #[derive(Default)]
    struct State {
        requests:Mutex<Vec<Arc<Mutex<Value>>>>,
        connections:Mutex<Vec<Arc<Mutex<Value>>>>,
        errors:Mutex<Vec<String>>,
        active:Mutex<usize>, idle:Condvar,
    }
    struct Active(Arc<State>);
    impl Drop for Active { fn drop(&mut self) { *self.0.active.lock().unwrap()-=1; self.0.idle.notify_all(); } }
    impl State {
        fn start(self: &Arc<Self>) -> Active { *self.active.lock().unwrap()+=1; Active(self.clone()) }
        fn record(&self, transport: &str, packet: &[u8], frame: &[u8]) -> Result<(String,Arc<Mutex<Value>>,Vec<u8>)> {
            let domain=question_name(packet)?;
            let mode=["end-header","end-body","malformed","timeout","fragment","extra","empty"].into_iter().find(|mode| domain.starts_with(mode)).unwrap_or(if domain=="fallback.fixture" && transport=="udp" { "empty" } else { "good" }).to_owned();
            let record=Arc::new(Mutex::new(json!({"transport":transport,"packet":packet,"frame":frame,"domain":domain})));
            self.requests.lock().unwrap().push(record.clone()); let response=response_bytes(packet,&mode)?;
            if mode=="timeout" { std::thread::sleep(Duration::from_millis(180)); }
            Ok((mode,record,response))
        }
        fn exchange<S:Read+Write>(&self, socket: &mut S, transport: &str, connection: &Arc<Mutex<Value>>, end_write: impl Fn(&S)) -> Result {
            let mut frame=[0;2]; socket.read_exact(&mut frame)?;
            let mut packet=vec![0;usize::from(u16::from_be_bytes(frame))]; socket.read_exact(&mut packet)?;
            let (mode,record,response)=self.record(transport,&packet,&frame)?;
            let mut framed=(response.len() as u16).to_be_bytes().to_vec(); framed.extend(response);
            match mode.as_str() { "end-header"=>framed.truncate(1),"end-body"=>framed.truncate(11),"extra"=>framed.extend([0,2,0,1]),_=>() }
            let chunks=if mode=="fragment" { vec![1,1,3,3,3,framed.len()-11] } else { vec![framed.len()] };
            record.lock().unwrap()["response_chunks"]=json!(chunks); let start=Instant::now(); let mut offset=0;
            for (index,size) in chunks.iter().enumerate() {
                if index>0 { std::thread::sleep(Duration::from_millis(60)); }
                if let Err(error)=socket.write_all(&framed[offset..offset+size]).and_then(|()| socket.flush()) {
                    if mode!="timeout" { return Err(error.into()); }
                    connection.lock().unwrap()["closed"]=json!(true);
                }
                offset+=size;
            }
            record.lock().unwrap()["response_elapsed"]=json!(start.elapsed().as_secs_f64());
            if mode.starts_with("end-") { end_write(socket); }
            let closed=match socket.read(&mut [0;1]) {
                Ok(count)=>count==0,
                Err(error) if [std::io::ErrorKind::ConnectionReset,std::io::ErrorKind::ConnectionAborted,std::io::ErrorKind::UnexpectedEof].contains(&error.kind())=>true,
                Err(error)=>return Err(error.into()),
            };
            connection.lock().unwrap()["closed"]=json!(closed); Ok(())
        }
        fn tcp(&self, mut socket: TcpStream, transport: &str, tls: Arc<tokio_rustls::rustls::ServerConfig>) -> Result {
            let connection=Arc::new(Mutex::new(json!({"transport":transport,"handshake":null,"server_name":null,"closed":false})));
            self.connections.lock().unwrap().push(connection.clone()); socket.set_read_timeout(Some(Duration::from_secs(10)))?; socket.set_write_timeout(Some(Duration::from_secs(10)))?;
            if transport=="tls" {
                let mut session=ServerConnection::new(tls)?; let start=Instant::now();
                let accepted=loop { if !session.is_handshaking() { break true; } if session.complete_io(&mut socket).is_err() { break false; } };
                {
                    let mut record=connection.lock().unwrap(); record["handshake"]=json!(accepted); record["server_name"]=json!(session.server_name()); record["handshake_elapsed"]=json!(start.elapsed().as_secs_f64());
                    record["tls_version"]=json!(session.protocol_version().map(|version| format!("{version:?}"))); record["cipher"]=json!(session.negotiated_cipher_suite().map(|suite| format!("{:?}",suite.suite())));
                }
                if !accepted { return Ok(()); }
                let mut stream=StreamOwned::new(session,socket);
                self.exchange(&mut stream,transport,&connection,|stream| { let _=stream.sock.shutdown(Shutdown::Write); })
            } else { self.exchange(&mut socket,transport,&connection,|stream| { let _=stream.shutdown(Shutdown::Write); }) }
        }
    }

    pub struct ControlledDns { pub urls:BTreeMap<String,String>, state:Arc<State>, stop:Arc<AtomicBool>, threads:Vec<JoinHandle<()>> }
    impl ControlledDns {
        pub fn new() -> Result<Self> {
            crate::actions_only(); let tls=tls_config()?;
            let mut fixture=Self { urls:BTreeMap::new(),state:Arc::default(),stop:Arc::new(AtomicBool::new(false)),threads:Vec::new() };
            for (transport,host,protocol) in [("udp","127.0.0.1","udp"),("udp6","::1","udp"),("tcp","127.0.0.1","tcp"),("tcp6","::1","tcp"),("tls","127.0.0.1","tls")] {
                let (state,stop,tls)=(fixture.state.clone(),fixture.stop.clone(),tls.clone());
                let (address,thread)=if protocol=="udp" {
                    let socket=Arc::new(UdpSocket::bind((host,0))?); socket.set_nonblocking(true)?; let address=socket.local_addr()?;
                    let thread=std::thread::spawn(move || {
                        let mut workers=Vec::new(); let mut buffer=vec![0;65536];
                        while !stop.load(Ordering::Acquire) {
                            match socket.recv_from(&mut buffer) {
                                Ok((count,peer))=>{ let (state,socket,packet)=(state.clone(),socket.clone(),buffer[..count].to_vec()); workers.push(std::thread::spawn(move || { let _active=state.start(); let result=state.record(transport,&packet,&[]).and_then(|(_,_,response)| { socket.send_to(&response,peer)?; Ok(()) }); if let Err(error)=result { state.errors.lock().unwrap().push(format!("{transport}: {error}")); } })); },
                                Err(error) if error.kind()==std::io::ErrorKind::WouldBlock=>std::thread::sleep(Duration::from_millis(2)),
                                Err(error)=>{ state.errors.lock().unwrap().push(error.to_string()); break; }
                            }
                        }
                        for worker in workers { if worker.join().is_err() { state.errors.lock().unwrap().push("DNS worker panicked".into()); } }
                    }); (address,thread)
                } else {
                    let listener=TcpListener::bind((host,0))?; listener.set_nonblocking(true)?; let address=listener.local_addr()?;
                    let thread=std::thread::spawn(move || {
                        let mut workers=Vec::new();
                        while !stop.load(Ordering::Acquire) {
                            match listener.accept() {
                                Ok((socket,_))=>{ let (state,tls)=(state.clone(),tls.clone()); workers.push(std::thread::spawn(move || { let _active=state.start(); if let Err(error)=state.tcp(socket,transport,tls) { state.errors.lock().unwrap().push(format!("{transport}: {error}")); } })); },
                                Err(error) if error.kind()==std::io::ErrorKind::WouldBlock=>std::thread::sleep(Duration::from_millis(2)),
                                Err(error)=>{ state.errors.lock().unwrap().push(error.to_string()); break; }
                            }
                        }
                        for worker in workers { if worker.join().is_err() { state.errors.lock().unwrap().push("DNS worker panicked".into()); } }
                    }); (address,thread)
                };
                fixture.urls.insert(transport.to_owned(),format!("{protocol}://{address}")); fixture.threads.push(thread);
            }
            for (name,key) in [("tlsName","tls"),("tcpName","tcp"),("udpName","udp")] { fixture.urls.insert(name.to_owned(),fixture.urls[key].replace("127.0.0.1","localhost")); }
            Ok(fixture)
        }
        pub fn wait_for_idle(&self) -> Result {
            let (active,_)=self.state.idle.wait_timeout_while(self.state.active.lock().unwrap(),Duration::from_secs(3),|active| *active!=0).unwrap();
            require(*active==0,"Owned DNS fixture still has active handlers")
        }
        pub fn requests(&self) -> Vec<Value> { self.state.requests.lock().unwrap().iter().map(|record| record.lock().unwrap().clone()).collect() }
        pub fn connections(&self) -> Vec<Value> { self.state.connections.lock().unwrap().iter().map(|record| record.lock().unwrap().clone()).collect() }
        pub fn errors(&self) -> Vec<String> { self.state.errors.lock().unwrap().clone() }
    }
    impl Drop for ControlledDns { fn drop(&mut self) { self.stop.store(true,Ordering::Release); for thread in self.threads.drain(..) { let _=thread.join(); } } }
}

pub use native::ControlledDns;

pub fn expected_doh_requests() -> Result<Vec<serde_json::Value>> {
    use serde_json::json;
    let mut queries=vec![("good","packet.fixture",1,Some("203.0.113.99")),("good","ipv6.fixture",28,Some("2001:db8:1234:5678::99"))];
    queries.extend([("status","status.fixture",1,None),("rcode","rcode.fixture",1,None),("malformed","malformed.fixture",1,None),("delay","timeout.fixture",1,None),("good","cache.fixture",1,Some("203.0.113.99"))]);
    queries.extend([("good","cache.fixture",1,None);3]);
    queries.push(("good","custom6.fixture",28,Some("2001:db8:1234:5678::99")));
    queries.extend([("empty","empty.fixture",1,None),("wrong","wrong.fixture",1,None),("nxdomain","nxdomain.fixture",1,None),("serial-empty","serial.fixture",1,None),("serial-good","serial.fixture",1,None),("race-slow","race.fixture",1,None),("race-fast","race.fixture",1,None),("release","release.fixture",1,None),("empty","exhausted.fixture",1,None),("wrong","exhausted.fixture",1,None),("good","injected.fixture",1,None),("good","injected-cache.fixture",1,None)]);
    queries.into_iter().map(|(mode,domain,kind,edns)| Ok(json!({"method":"GET","path":query_path(mode,domain,kind,edns.map(str::parse).transpose()?)?,"body_bytes":[],"accept":"application/dns-message"}))).collect()
}

pub fn check_doh_requests(requests: &[serde_json::Value]) -> Result {
    use serde_json::json;
    require(requests.len()>=18,"Missing DNS-over-HTTPS requests")?;
    let mut expected=expected_doh_requests()?;
    let mut actual=requests.iter().map(|item| json!({"method":item["method"],"path":item["path"],"body_bytes":item["body_bytes"],"accept":item["headers"].get("accept")})).collect::<Vec<_>>();
    for rows in [&mut actual,&mut expected] { rows[16..18].sort_by(|left,right| left["path"].as_str().cmp(&right["path"].as_str())); }
    crate::json::check(&json!(actual),&json!(expected),false)?;
    require(requests[16..18].iter().all(|item| item["dns_overlap"]==true),"DNS racers must overlap before either responds")
}

pub fn expected_native_requests() -> Result<Vec<serde_json::Value>> {
    use serde_json::json;
    let queries=[("udp","udp-a.fixture",1,Some("203.0.113.99")),("udp","udp-hostname.fixture",1,None),("udp6","udp6-aaaa.fixture",28,Some("2001:db8:1234:5678::99")),("tcp","hostname-tcp.fixture",1,None),("tcp","fragment-tcp.fixture",1,None),("tcp6","extra-tcp6.fixture",28,None),("tcp","end-header-tcp.fixture",1,None),("tcp","end-body-tcp.fixture",1,None),("udp","malformed-udp.fixture",1,None),("tcp","malformed-tcp.fixture",1,None),("udp","timeout-udp.fixture",1,None),("tcp","timeout-tcp.fixture",1,None),("tls","tls-hostname.fixture",1,None),("tls","tls-ip.fixture",28,None),("tls","fragment-tls.fixture",1,None),("tls","end-body-tls.fixture",1,None),("tls","timeout-tls.fixture",1,None),("udp","fallback.fixture",1,None),("tcp","fallback.fixture",1,None),("tls","tls-cache.fixture",1,None),("tcp","injected-native.fixture",1,None)];
    queries.into_iter().map(|(transport,domain,kind,edns)| { let packet=query_bytes(domain,kind,edns.map(str::parse).transpose()?)?; let frame=if transport.starts_with("udp") { Vec::new() } else { (packet.len() as u16).to_be_bytes().to_vec() }; Ok(json!({"transport":transport,"domain":domain,"packet":packet,"frame":frame})) }).collect()
}

pub fn expected_connections() -> Vec<serde_json::Value> {
    use serde_json::json;
    let mut connections=vec![("tcp",None,None),("tcp",None,None),("tcp6",None,None)];
    connections.extend([("tcp",None,None);4]);
    connections.extend([("tls",Some(false),Some("localhost")),("tls",Some(false),Some("localhost")),("tls",Some(true),Some("localhost"))]);
    connections.extend([("tls",Some(true),None);4]);
    connections.extend([("tcp",None,None),("tls",Some(true),Some("localhost")),("tls",Some(false),Some("localhost")),("tcp",None,None)]);
    connections.into_iter().map(|(transport,handshake,server_name)| json!({"transport":transport,"handshake":handshake,"server_name":server_name})).collect()
}

pub fn check_native(requests: &[serde_json::Value], connections: &[serde_json::Value], errors: &[String]) -> Result {
    use serde_json::{Value,json};
    require(errors.is_empty(),"Controlled DNS fixture failed")?;
    require(connections.iter().all(|item| ["transport","handshake","server_name"].iter().all(|key| item.get(key).is_some())),"Missing DNS connection evidence")?;
    let actual=requests.iter().map(|item| json!({"transport":item["transport"],"domain":item["domain"],"packet":item["packet"],"frame":item["frame"]})).collect::<Vec<_>>();
    crate::json::check(&json!(actual),&json!(expected_native_requests()?),false)?;
    let actual=connections.iter().map(|item| json!({"transport":item["transport"],"handshake":item["handshake"],"server_name":item["server_name"]})).collect::<Vec<_>>();
    crate::json::check(&json!(actual),&json!(expected_connections()),false)?;
    let closed=connections.iter().filter(|item| item["handshake"]!=false).map(|item| item.get("closed").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>();
    crate::json::check(&json!(closed),&json!(vec![true;15]),false)?;
    for item in requests {
        if item["domain"].as_str().is_some_and(|domain| domain.starts_with("fragment-")) {
            let packet=item["packet"].as_array().ok_or("Missing DNS packet")?.iter().map(|byte| byte.as_u64().and_then(|value| u8::try_from(value).ok()).ok_or("Invalid DNS packet byte")).collect::<std::result::Result<Vec<_>,_>>()?;
            let size=response_bytes(&packet,"good")?.len()+2;
            crate::json::check(&item["response_chunks"],&json!([1,1,3,3,3,size-11]),false)?;
            require(item["response_elapsed"].as_f64().is_some_and(|seconds| seconds>=0.28),"Fragment fixture must outlast 200ms inactivity timeout")?;
        }
    }
    Ok(())
}
