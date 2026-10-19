use crate::conversion::{Error, field_text as text, vmess_security, wireguard_address};
use crate::json as codec;
use crate::state::{legacy_object_order, truthy};
use serde_json::{Map, Value, json};

fn get<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value, |value, key| value.get(key))
}

fn copy(out: &mut Map<String, Value>, proxy: &Value, fields: &[(&str, &str)]) {
    for (from, to) in fields {
        if let Some(value) = get(proxy, from) {
            out.insert((*to).to_owned(), value.clone());
        }
    }
}

fn first(value: Option<&Value>) -> Option<Value> {
    value.and_then(|value| value.as_array().map_or(Some(value), |items|items.first())).cloned()
}

fn tfo(out: &mut Map<String, Value>, proxy: &Value) {
    let value = proxy.get("tfo").filter(|value|!value.is_null()).or_else(||proxy.get("fast-open"));
    out.insert("tfo".to_owned(), json!(value.is_some_and(truthy)));
}

fn udp(out: &mut Map<String, Value>, proxy: &Value) {
    if let Some(value) = proxy.get("udp").filter(|value|!value.is_null()).or_else(||proxy.get("udp_relay")) {
        out.insert("udp_relay".to_owned(), value.clone());
    }
}

fn normalized(value: Option<&Value>) -> Option<String> {
    value.filter(|value|!value.is_null()).map(codec::js_string).map(|value|codec::trim(&value).to_lowercase()).filter(|value|!value.is_empty())
}

fn ssr_method(proxy: &Value) -> Option<String> {
    normalized(proxy.get("cipher")).map(|value| if value == "plain" {"none".to_owned()}else{value})
}

fn snell_version(proxy: &Value) -> Result<Option<Value>, ()> {
    let Some(value) = proxy.get("version").filter(|value|!value.is_null()) else {return Ok(None)};
    let value=codec::js_string(value);
    let value=codec::trim(&value);
    if value.len()!=1 || !matches!(value.as_bytes()[0], b'1'..=b'5') {return Err(())}
    Ok(Some(json!(value.as_bytes()[0]-b'0')))
}

fn supported(proxy: &Value) -> bool {
    let kind=text(proxy,"type");
    if !["http","https","socks5","ss","ssr","trojan","hysteria2","vless","vmess","tuic","wireguard","anytls","ssh","snell"].contains(&kind.as_str()) {return false}
    if kind=="ss" && ((proxy["plugin"]=="obfs" && !["http","tls"].contains(&proxy["plugin-opts"]["mode"].as_str().unwrap_or(""))) ||
        !["chacha20-ietf-poly1305","chacha20-poly1305","aes-256-gcm","aes-128-gcm","none","tbale","rc4","rc4-md5","aes-128-cfb","aes-192-cfb","aes-256-cfb","aes-128-ctr","aes-192-ctr","aes-256-ctr","bf-cfb","camellia-128-cfb","camellia-192-cfb","camellia-256-cfb","cast5-cfb","des-cfb","idea-cfb","rc2-cfb","seed-cfb","salsa20","chacha20","chacha20-ietf","2022-blake3-aes-128-gcm","2022-blake3-aes-256-gcm"].contains(&proxy["cipher"].as_str().unwrap_or(""))) {return false}
    if ["vmess","vless"].contains(&kind.as_str()) {
        if truthy(&proxy["network"]) && !["h2","http","ws","tcp","grpc"].contains(&proxy["network"].as_str().unwrap_or("")) {return false}
        if proxy["network"]=="grpc" && get(proxy,"grpc-opts._grpc-type").is_some_and(|value| !value.is_null() && codec::trim(&codec::js_string(value)).to_lowercase()!="gun") {return false}
    }
    if kind=="trojan" && truthy(&proxy["network"]) && !["http","ws","tcp"].contains(&proxy["network"].as_str().unwrap_or("")) {return false}
    if kind=="vless" && proxy.get("flow").is_some_and(|flow| flow!="" && flow!="xtls-rprx-vision") {return false}
    if kind=="tuic" && truthy(&proxy["token"]) && !(proxy["token"].as_str().is_some_and(str::is_empty) || proxy["token"].as_array().is_some_and(Vec::is_empty) || proxy["token"]["length"]==0) {return false}
    if kind=="snell" && snell_version(proxy).is_err() {return false}
    if kind=="ssr" {
        if !ssr_method(proxy).is_some_and(|value| ["none","dummy","rc4-md5","aes-128-cfb","aes-192-cfb","aes-256-cfb","aes-128-ctr","aes-192-ctr","aes-256-ctr","chacha20","chacha20-ietf","xchacha20"].contains(&value.as_str())) {return false}
        if normalized(proxy.get("protocol")).is_some_and(|value| !["origin","auth_sha1_v4","auth_aes128_md5","auth_aes128_sha1","auth_chain_a","auth_chain_b"].contains(&value.as_str())) {return false}
        if normalized(proxy.get("obfs")).is_some_and(|value| !["plain","http_simple","http_post","random_head","tls1.2_ticket_auth","tls1.2_ticket_fastauth"].contains(&value.as_str())) {return false}
    }
    if kind=="anytls" && truthy(&proxy["network"]) && proxy["network"]!="tcp" {return false}
    !(proxy["network"]=="ws" && truthy(&proxy["ws-opts"]["v2ray-http-upgrade"]))
}

fn reality(proxy: &Value) -> Option<Value> {
    if !truthy(&proxy["reality-opts"]) {return None}
    let mut out=Map::new();
    for (from,to) in [("public-key","public_key"),("short-id","short_id")] {
        if let Some(value)=proxy["reality-opts"].get(from).filter(|value|!value.is_null() && **value!="") {
            out.insert(to.to_owned(),value.clone());
        }
    }
    (!out.is_empty()).then_some(Value::Object(out))
}

fn tls(out: &mut Map<String, Value>, proxy: &Value) {
    copy(out,proxy,&[("sni","sni"),("skip-cert-verify","skip_tls_verify")]);
    if let Some(value)=reality(proxy) {out.insert("reality".to_owned(),value);}
}

fn h2_headers(proxy: &Value) -> Option<Value> {
    let mut out=Map::new();
    if let Some(headers)=proxy["h2-opts"]["headers"].as_object() {
        for (key,value) in legacy_object_order(Value::Object(headers.clone())).as_object().unwrap() {
            if !key.eq_ignore_ascii_case("host") && let Some(value)=first(Some(value)).filter(|value|!value.is_null()) {
                out.insert(key.clone(),value);
            }
        }
    }
    let host=first(get(proxy,"h2-opts.host")).filter(truthy)
        .or_else(||first(get(proxy,"h2-opts.headers.host")).filter(truthy))
        .or_else(||first(get(proxy,"h2-opts.headers.Host")).filter(truthy));
    if let Some(value)=host {out.insert("Host".to_owned(),value);}
    (!out.is_empty()).then_some(Value::Object(out))
}

fn transport(proxy: &Value, vless: bool) -> Option<Value> {
    let mut fields=Map::new();
    let network=text(proxy,"network");
    let kind=match network.as_str() {
        "ws"=>{
            copy(&mut fields,proxy,&[("ws-opts.path","path")]);
            let mut headers=Map::new();copy(&mut headers,proxy,&[("ws-opts.headers.Host","Host")]);
            fields.insert("headers".to_owned(),Value::Object(headers));
            if truthy(&proxy["tls"]) {copy(&mut fields,proxy,&[("sni","sni"),("skip-cert-verify","skip_tls_verify")]);}
            if truthy(&proxy["tls"]) {"wss"}else{"ws"}
        },
        "http"=>{
            copy(&mut fields,proxy,&[("http-opts.method","method")]);
            if let Some(value)=first(get(proxy,"http-opts.path")) {fields.insert("path".to_owned(),value);}
            let mut headers=Map::new();if let Some(value)=first(get(proxy,"http-opts.headers.Host")) {headers.insert("Host".to_owned(),value);}
            fields.insert("headers".to_owned(),Value::Object(headers));
            copy(&mut fields,proxy,&[("skip-cert-verify","skip_tls_verify")]);
            "http1"
        },
        "h2"=>{
            copy(&mut fields,proxy,&[("h2-opts.method","method")]);
            if let Some(value)=first(get(proxy,"h2-opts.path")) {fields.insert("path".to_owned(),value);}
            if let Some(value)=h2_headers(proxy) {fields.insert("headers".to_owned(),value);}
            copy(&mut fields,proxy,&[("sni","sni"),("skip-cert-verify","skip_tls_verify")]);
            "http2"
        },
        "grpc"=>{
            copy(&mut fields,proxy,&[("grpc-opts.grpc-service-name","service_name"),("sni","sni")]);
            if let Some(value)=reality(proxy) {fields.insert("reality".to_owned(),value);}
            copy(&mut fields,proxy,&[("skip-cert-verify","skip_tls_verify")]);
            "grpc"
        },
        _ if (network=="tcp" || !truthy(&proxy["network"])) && (vless || truthy(&proxy["tls"]))=>{
            if truthy(&proxy["tls"]) {copy(&mut fields,proxy,&[("sni","sni"),("skip-cert-verify","skip_tls_verify")]);}
            if vless && let Some(value)=reality(proxy) {fields.insert("reality".to_owned(),value);}
            if truthy(&proxy["tls"]) {"tls"}else{"tcp"}
        },
        _=>return proxy.get("transport").cloned(),
    };
    Some(json!({kind:Value::Object(fields)}))
}

fn list_text(value: Option<&Value>, separator: char) -> Result<Option<Value>,()> {
    let Some(value)=value.filter(|value|truthy(value)) else {return Ok(None)};
    if value.is_array() {return Ok(Some(value.clone()))}
    let raw=value.as_str().ok_or(())?;
    Ok(Some(json!(raw.split(separator).map(codec::trim).filter(|value|!value.is_empty()).collect::<Vec<_>>())))
}

fn empty_transport(value: &Value) -> Result<bool,()> {
    if value.is_null() {return Err(())}
    Ok(crate::body::Body::Json(value.clone()).fields().values().all(Value::is_null))
}

fn convert(mut proxy: Value, kind: &str) -> Result<Value,()> {
    let original=proxy.clone();
    let source_type=text(&proxy,"type");
    if truthy(&proxy["tls"]) && !truthy(&proxy["sni"]) {
        if let Some(value)=proxy.get("server").cloned() {proxy["sni"]=value;}else{proxy.as_object_mut().unwrap().shift_remove("sni");}
    }
    let prev_hop=["prev_hop","underlying-proxy","dialer-proxy"].iter().find_map(|key|proxy.get(key).filter(|value|truthy(value)).cloned()).or_else(||proxy.get("detour").cloned());
    let mut out=Map::new();
    let output_type=match source_type.as_str() {
        "ss"=>"shadowsocks","ssr"=>"shadowsocksr",
        "http" if truthy(&proxy["tls"])=>"https",
        "socks5" if truthy(&proxy["tls"])=>"socks5_tls",value=>value,
    };
    copy(&mut out,&proxy,&[("name","name")]);
    if ["ss","ssr"].contains(&source_type.as_str()) {
        let method=if source_type=="ssr" {ssr_method(&proxy).map(Value::String)}
            else if proxy["cipher"]=="chacha20-ietf-poly1305" {Some(json!("chacha20-poly1305"))}else{proxy.get("cipher").cloned()};
        if let Some(value)=method {out.insert("method".to_owned(),value);}
    }
    if source_type=="wireguard" {
        if let Some(peer)=proxy["peers"].as_array().and_then(|peers|peers.first()).cloned() {
            if peer.is_null() {return Err(())}
            for (from,to) in [("server","server"),("port","port"),("ip","ip"),("ipv6","ipv6"),("public-key","public-key"),("pre-shared-key","preshared-key"),("allowed-ips","allowed-ips"),("reserved","reserved")] {
                if let Some(value)=peer.get(from) {proxy[to]=value.clone();}else{proxy.as_object_mut().unwrap().shift_remove(to);}
            }
        }
        for (family,key) in [("ipv4","local_ipv4"),("ipv6","local_ipv6")] {
            if let Some(value)=wireguard_address(&proxy,family) {out.insert(key.to_owned(),value);}
        }
    }
    copy(&mut out,&proxy,&[("server","server"),("port","port")]);
    match source_type.as_str() {
        "http"|"https"|"socks5"=>{
            copy(&mut out,&proxy,&[("username","username"),("password","password")]);
            if source_type!="socks5" && (proxy["headers"].as_object().is_some_and(|v|!v.is_empty())||proxy["headers"].as_array().is_some_and(|v|!v.is_empty())) {out.insert("headers".to_owned(),proxy["headers"].clone());}
            tfo(&mut out,&proxy);
            if source_type=="socks5" {udp(&mut out,&proxy);}
            if source_type=="https"||truthy(&proxy["tls"]) {tls(&mut out,&proxy);}
        },
        "ss"=>{
            copy(&mut out,&proxy,&[("password","password")]);tfo(&mut out,&proxy);udp(&mut out,&proxy);
            if proxy.get("plugin").is_some_and(|v|!v.is_null()) {
                if proxy["plugin"]=="obfs" {
                    if proxy["plugin-opts"].is_null() {return Err(())}
                    copy(&mut out,&proxy,&[("plugin-opts.mode","obfs"),("plugin-opts.host","obfs_host"),("plugin-opts.path","obfs_uri")]);
                }else if proxy["plugin"]!="shadow-tls" {return Err(())}
            }
        },
        "ssr"=>{
            copy(&mut out,&proxy,&[("password","password")]);
            if let Some(value)=normalized(proxy.get("protocol")) {out.insert("protocol".to_owned(),json!(value));}
            copy(&mut out,&proxy,&[("protocol-param","protocol_param")]);
            if let Some(value)=normalized(proxy.get("obfs")) {out.insert("obfs".to_owned(),json!(value));}
            copy(&mut out,&proxy,&[("obfs-param","obfs_param")]);tfo(&mut out,&proxy);udp(&mut out,&proxy);
        },
        "hysteria2"=>{
            copy(&mut out,&proxy,&[("password","auth")]);
            if let Some(value)=proxy.get("up").filter(|v|!v.is_null()) {
                let raw=codec::js_string(value);
                let number:String=raw.chars().skip_while(|c|!c.is_ascii_digit()).take_while(char::is_ascii_digit).collect();
                out.insert("bandwidth".to_owned(),json!(number.parse::<f64>().unwrap_or(0.0)));
            }
            tfo(&mut out,&proxy);udp(&mut out,&proxy);
            copy(&mut out,&proxy,&[("sni","sni"),("skip-cert-verify","skip_tls_verify"),("ports","port_hopping"),("hop-interval","port_hopping_interval")]);
            if truthy(&proxy["obfs-password"]) && proxy["obfs"]=="salamander" {
                out.insert("obfs".to_owned(),json!("salamander"));out.insert("obfs_password".to_owned(),proxy["obfs-password"].clone());
            }
        },
        "tuic"=>{
            copy(&mut out,&proxy,&[("uuid","uuid"),("password","password"),("sni","sni")]);
            out.insert("alpn".to_owned(),if proxy["alpn"].is_array(){proxy["alpn"].clone()}else{json!([proxy.get("alpn").filter(|v|truthy(v)).cloned().unwrap_or(json!("h3"))])});
            copy(&mut out,&proxy,&[("skip-cert-verify","skip_tls_verify"),("ports","port_hopping"),("hop-interval","port_hopping_interval")]);
        },
        "trojan"|"anytls"=>{
            copy(&mut out,&proxy,&[("password","password")]);tfo(&mut out,&proxy);udp(&mut out,&proxy);tls(&mut out,&proxy);
            if source_type=="trojan" {
                if proxy["network"]=="ws" {let mut ws=Map::new();copy(&mut ws,&proxy,&[("ws-opts.path","path"),("ws-opts.headers.Host","host")]);out.insert("websocket".to_owned(),Value::Object(ws));}
                else {copy(&mut out,&proxy,&[("websocket","websocket")]);}
            }
        },
        "vmess"|"vless"=>{
            if source_type=="vless" && truthy(&proxy["encryption"]) && proxy["encryption"]!="none" {return Err(())}
            copy(&mut out,&proxy,&[("uuid","user_id")]);
            if source_type=="vmess" {out.insert("security".to_owned(),vmess_security(&proxy["cipher"]));}else{copy(&mut out,&proxy,&[("cipher","security")]);}
            tfo(&mut out,&proxy);
            if source_type=="vmess" {let legacy=proxy.get("aead").is_some_and(|v|!v.is_null()&&!truthy(v)) || proxy.get("alterId").is_some_and(|v|!v.is_null()&&*v!=0);out.insert("legacy".to_owned(),json!(legacy));}
            udp(&mut out,&proxy);
            if let Some(value)=transport(&proxy,source_type=="vless") {out.insert("transport".to_owned(),value);}
            if source_type=="vless" && (proxy["network"]=="tcp"||!truthy(&proxy["network"])) && proxy["flow"]!="" {copy(&mut out,&proxy,&[("flow","flow")]);}
        },
        "wireguard"=>{
            copy(&mut out,&proxy,&[("private-key","private_key"),("public-key","peer_public_key"),("preshared-key","preshared_key")]);
            if let Some(value)=list_text(proxy.get("reserved"),'/')? {out.insert("reserved".to_owned(),value);}
            if let Some(value)=list_text(proxy.get("dns"),',')? {out.insert("dns_servers".to_owned(),value);}
            copy(&mut out,&proxy,&[("mtu","mtu"),("keepalive","keepalive")]);
        },
        "ssh"=>{
            copy(&mut out,&proxy,&[("username","username"),("password","password"),("private-key","private_key"),("host-key","host_keys")]);tfo(&mut out,&proxy);
        },
        "snell"=>{
            copy(&mut out,&proxy,&[("psk","psk")]);let version=snell_version(&proxy)?;
            if let Some(value)=&version {out.insert("version".to_owned(),value.clone());}
            if version.as_ref().is_none_or(|v|codec::to_number(v)>=3.0) {udp(&mut out,&proxy);}
            copy(&mut out,&proxy,&[("reuse","reuse")]);
            for (paths,to) in [(&["obfs-opts.mode","obfs"][..],"obfs"),(&["obfs-opts.host","obfs-host","obfs_host"][..],"obfs_host")] {
                if let Some(value)=paths.iter().find_map(|path|get(&proxy,path).filter(|v|truthy(v)).cloned()){out.insert(to.to_owned(),value);}
            }
            tfo(&mut out,&proxy);
        },
        _=>return Err(()),
    }
    if ["http","https","socks5","ss","ssr","trojan","vless","vmess","anytls","ssh","snell"].contains(&source_type.as_str()) && proxy["plugin"]=="shadow-tls" && truthy(&proxy["plugin-opts"]) {
        if codec::to_number(&proxy["plugin-opts"]["version"])!=3.0 {return Err(())}
        let mut fields=Map::new();copy(&mut fields,&proxy,&[("plugin-opts.password","password"),("plugin-opts.host","sni")]);out.insert("shadow_tls".to_owned(),Value::Object(fields));
    }
    if let Some(fingerprint)=original["tls-fingerprint"].as_str().map(codec::trim).filter(|v|!v.is_empty()) {
        if ["anytls","https","hysteria2","trojan","tuic"].contains(&source_type.as_str()) || (source_type=="socks5"&&output_type=="socks5_tls") || (source_type=="http"&&output_type=="https") {out.insert("fingerprint_sha256".to_owned(),json!(fingerprint));}
        if let Some(transports)=out.get_mut("transport").and_then(Value::as_object_mut) {for key in ["grpc","http2","tls","wss"] {if let Some(fields)=transports.get_mut(key).and_then(Value::as_object_mut) {fields.insert("fingerprint_sha256".to_owned(),json!(fingerprint));}}}
    }
    if ["socks5","ss","ssr","trojan","vless","vmess","wireguard","tuic","hysteria2","anytls","ssh","snell"].contains(&source_type.as_str()) {
        let value=&original["block-quic"];
        if [json!("on"),json!("true"),json!(true),json!("1"),json!(1)].contains(value) {out.insert("block_quic".to_owned(),json!(true));}
        else if [json!("off"),json!("false"),json!(false),json!("0"),json!(0)].contains(value) {out.insert("block_quic".to_owned(),json!(false));}
    }
    if ["ss","ssr"].contains(&source_type.as_str()) && out.contains_key("shadow_tls") && codec::to_number(&original["udp-port"])>0.0 && codec::to_number(&original["udp-port"])<=65535.0 {out.insert("udp_port".to_owned(),original["udp-port"].clone());}
    if truthy(&original["ip-version"]) {
        let raw=text(&original,"ip-version");
        let mapped=match raw.as_str() {"dual"=>"dual_stack","ipv4"|"v4-only"=>"v4_only","ipv6"|"v6-only"=>"v6_only","ipv4-prefer"|"prefer-v4"=>"v4_prefer","ipv6-prefer"|"prefer-v6"=>"v6_prefer",_=>&raw};
        out.insert("ip_version".to_owned(),if mapped==raw{original["ip-version"].clone()}else{json!(mapped)});
    }
    if let Some(transport)=out.get_mut("transport").filter(|value|truthy(value)) {
        let remove=match transport {
            Value::Object(fields)=>{
                let mut remove=Vec::new();
                for (key,value) in fields.iter() {
                    if key!="grpc" && empty_transport(value)? {remove.push(key.clone());}
                }
                for key in remove {fields.shift_remove(&key);}
                fields.is_empty()
            },
            Value::Array(values)=>{
                let mut kept=0;
                for value in values {
                    if empty_transport(value)? {*value=Value::Null;}else{kept+=1;}
                }
                kept==0
            },
            Value::String(_)=>false,
            _=>true,
        };
        if remove {out.shift_remove("transport");}
    }
    if kind!="internal" {out.retain(|key,value|!value.is_null()&&!key.starts_with('_'));}
    if let Some(value)=prev_hop {out.insert("prev_hop".to_owned(),value);}
    Ok(json!({output_type:Value::Object(out)}))
}

pub fn produce(proxies: Vec<Value>, kind: &str, opts: &Value) -> Result<Value, Error> {
    let list=proxies.into_iter().filter(supported).filter_map(|proxy|convert(proxy,kind).ok()).collect();
    crate::clash_output::finish(list,kind,opts)
}
