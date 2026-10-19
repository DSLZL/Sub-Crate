use crate::conversion::{Error, field_text as text, vmess_security, wireguard_address};
use crate::json as codec;
use crate::state::{legacy_object_order, truthy};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};

fn copy(out: &mut Value, source: &Value, fields: &[(&str, &str)]) {
    for (from,to) in fields {if let Some(value)=source.get(from) {out[*to]=value.clone();}}
}

fn fields(value: &Value) -> Map<String,Value> {
    crate::body::Body::Json(legacy_object_order(value.clone())).fields()
}

fn integer(value: Option<&Value>) -> f64 {
    let raw=value.map(codec::js_string).unwrap_or_else(||"undefined".to_owned());
    let raw=codec::trim(&raw);
    let end=raw.char_indices().skip(usize::from(raw.starts_with(['+','-']))).find(|(_,c)|!c.is_ascii_digit()).map_or(raw.len(),|(index,_)|index);
    raw[..end].parse().unwrap_or(f64::NAN)
}

fn safe_integer(value: Option<&Value>) -> Option<u64> {
    let raw=value.filter(|value|!value.is_null()).map(codec::js_string)?;
    if raw.is_empty()||!raw.bytes().all(|c|c.is_ascii_digit()){return None}
    raw.parse().ok().filter(|value|*value<=9_007_199_254_740_991)
}

fn digits(value: Option<&Value>) -> bool {
    let raw=value.map(codec::js_string).unwrap_or_else(||"undefined".to_owned());
    !raw.is_empty()&&raw.bytes().all(|c|c.is_ascii_digit())
}

fn has_length(value: &Value) -> bool {
    match value {Value::String(value)=>!value.is_empty(),Value::Array(value)=>!value.is_empty(),_=>truthy(&value["length"])}
}

fn base(proxy: &Value, kind: &str) -> Result<Value,()> {
    let port=integer(proxy.get("port"));
    if kind!="anytls"&&(port<0.0||port>65535.0) {return Err(())}
    let mut out=json!({});
    copy(&mut out,proxy,&[("name","tag")]);out["type"]=json!(kind);
    copy(&mut out,proxy,&[("server","server")]);out["server_port"]=json!(port);
    Ok(out)
}

fn initial_tls(proxy: &Value, out: &mut Value, enabled: bool) {
    let mut tls=json!({"enabled":enabled});
    copy(&mut tls,proxy,&[("server","server_name")]);tls["insecure"]=json!(false);out["tls"]=tls;
}

fn network(proxy: &Value, out: &mut Value) {
    if proxy["_network"].as_str().is_some_and(|value|["tcp","udp"].contains(&value)) {out["network"]=proxy["_network"].clone();}
    else if proxy["udp"]==false {out["network"]=json!("tcp");}
}

fn tfo(proxy: &Value, out: &mut Value) {
    if ["tfo","tcp_fast_open","tcp-fast-open"].iter().any(|key|truthy(&proxy[*key])) {out["tcp_fast_open"]=json!(true);}
}

fn fragment(proxy: &Value, out: &mut Value) {
    if truthy(&proxy["fast-open"]) {out["udp_fragment"]=json!(true);}
}

fn detour(proxy: &Value, out: &mut Value) {
    if truthy(&proxy["dialer-proxy"]) {out["detour"]=proxy["dialer-proxy"].clone();}
    else {copy(out,proxy,&[("detour","detour")]);}
}

fn resolver(proxy: &Value, out: &mut Value) {
    let strategy=match text(proxy,"ip-version").as_str() {
        "ipv4"|"v4-only"=>Some("ipv4_only"),"ipv6"|"v6-only"=>Some("ipv6_only"),
        "ipv4-prefer"|"prefer-v4"=>Some("prefer_ipv4"),"ipv6-prefer"|"prefer-v6"=>Some("prefer_ipv6"),_=>None,
    };
    if let Some(strategy)=strategy && truthy(&proxy["_dns_server"]) {
        out["domain_resolver"]=json!({"server":proxy["_dns_server"],"strategy":strategy});
    }
    if truthy(&proxy["_domain_resolver"]) {
        let mut result=fields(&out["domain_resolver"]);
        if proxy["_domain_resolver"].is_string() {result.insert("server".to_owned(),proxy["_domain_resolver"].clone());}
        else {result.extend(fields(&proxy["_domain_resolver"]));}
        out["domain_resolver"]=Value::Object(result);
    }
}

fn smux(proxy: &Value, out: &mut Value) {
    let raw=&proxy["smux"];
    if !truthy(raw)||!truthy(&raw["enabled"]) {return}
    let mut mux=json!({"enabled":true});copy(&mut mux,raw,&[("protocol","protocol")]);
    for (from,to) in [("max-connections","max_connections"),("max-streams","max_streams"),("min-streams","min_streams")] {
        if truthy(&raw[from]) {mux[to]=json!(integer(raw.get(from)));}
    }
    if truthy(&raw["padding"]) {mux["padding"]=json!(true);}
    if truthy(&raw["brutal-opts"]["up"])||truthy(&raw["brutal-opts"]["down"]) {
        let mut brutal=json!({"enabled":true});
        for (from,to) in [("up","up_mbps"),("down","down_mbps")] {
            if truthy(&raw["brutal-opts"][from]) {brutal[to]=json!(integer(raw["brutal-opts"].get(from)));}
        }
        mux["brutal"]=brutal;
    }
    out["multiplex"]=mux;
}

fn udp_over_tcp(proxy: &Value, out: &mut Value) {
    if truthy(&proxy["uot"]) {out["udp_over_tcp"]=json!(true);}
    if truthy(&proxy["udp-over-tcp"]) {
        out["udp_over_tcp"]=json!({"enabled":true,"version":if !truthy(&proxy["udp-over-tcp-version"])||proxy["udp-over-tcp-version"]==1 {1}else{2}});
    }
}

fn certificate(proxy: &Value, tls: &mut Value) -> Result<(),()> {
    if truthy(&proxy["ca"]) {tls["certificate_path"]=json!(text(proxy,"ca"));}
    for from in ["ca_str","ca-str"] {if truthy(&proxy[from]) {tls["certificate"]=json!([proxy[from]]);}}
    for (from,to) in [("_certificate","certificate"),("_certificate_path","certificate_path")] {if truthy(&proxy[from]) {tls[to]=proxy[from].clone();}}
    if truthy(&tls["reality"]["enabled"]) {return Ok(())}
    if truthy(&proxy["_certificate_public_key_sha256"]) {tls["certificate_public_key_sha256"]=proxy["_certificate_public_key_sha256"].clone();}
    let has_certificate=has_length(&tls["certificate"])||truthy(&tls["certificate_path"]);
    if let Some(value)=proxy.get("_certificate_sha256").filter(|value|!value.is_null()) {tls["certificate_sha256"]=value.clone();}
    else if (truthy(&proxy["fingerprint"])||truthy(&proxy["tls-fingerprint"]))&&!has_certificate&&!has_length(&tls["certificate_public_key_sha256"]) {
        let value=if truthy(&proxy["fingerprint"]) {&proxy["fingerprint"]}else{&proxy["tls-fingerprint"]};
        let hex=codec::trim(&codec::js_string(value)).replace(':',"");
        if hex.len()!=64||!hex.bytes().all(|c|c.is_ascii_hexdigit()) {return Err(())}
        let bytes:Vec<u8>=hex.as_bytes().chunks_exact(2).map(|pair|u8::from_str_radix(std::str::from_utf8(pair).unwrap(),16).unwrap()).collect();
        tls["certificate_sha256"]=json!([STANDARD.encode(bytes)]);
    }
    if has_certificate&&(has_length(&tls["certificate_sha256"])||has_length(&tls["certificate_public_key_sha256"])) {return Err(())}
    Ok(())
}

fn fingerprint(value: &Value) -> Option<String> {
    let raw=if truthy(value) {codec::js_string(value)}else{String::new()};
    let raw=codec::trim(&raw).to_lowercase();
    ["chrome","firefox","edge","safari","360","qq","ios","android","random","randomized"].contains(&raw.as_str()).then_some(raw)
}

fn pem(value: &Value) -> Option<Value> {
    let values=value.as_array().map_or_else(||vec![value],|values|values.iter().collect());
    let mut lines=Vec::new();
    for value in values {
        let raw=codec::trim(&codec::js_string(value)).replace("\\r\\n","\n").replace("\\n","\n");
        for line in raw.split('\n').map(codec::trim).filter(|line|!line.is_empty()) {lines.push(line.to_owned());}
    }
    if lines.is_empty() {return None}
    let has_begin=lines.iter().any(|line|line.strip_prefix("-----BEGIN ").and_then(|line|line.strip_suffix("-----")).is_some_and(|label|!label.is_empty()&&label.bytes().all(|c|c.is_ascii_alphanumeric()||c==b' '||c==b'-')));
    if !has_begin {lines.insert(0,"-----BEGIN ECH CONFIGS-----".to_owned());lines.push("-----END ECH CONFIGS-----".to_owned());}
    Some(json!(lines))
}

fn tls(proxy: &Value, out: &mut Value) -> Result<(),()> {
    let tls=&mut out["tls"];
    if truthy(&proxy["tls"]) {tls["enabled"]=json!(true);}
    for key in ["servername","peer","sni"] {if truthy(&proxy[key]) {tls["server_name"]=proxy[key].clone();}}
    if truthy(&proxy["skip-cert-verify"])||truthy(&proxy["insecure"]) {tls["insecure"]=json!(true);}
    if truthy(&proxy["disable-sni"]) {tls["disable_sni"]=json!(true);}
    if proxy["alpn"].is_string() {tls["alpn"]=json!([proxy["alpn"]]);}
    else if proxy["alpn"].is_array() {tls["alpn"]=proxy["alpn"].clone();}
    if truthy(&proxy["reality-opts"]) {
        let raw=&proxy["reality-opts"];let mut reality=json!({"enabled":true});
        for (from,to) in [("public-key","public_key"),("short-id","short_id")] {if truthy(&raw[from]) {reality[to]=raw[from].clone();}}
        tls["reality"]=reality;tls["utls"]=json!({"enabled":true});
    }
    if !["hysteria","hysteria2","tuic"].contains(&text(proxy,"type").as_str())
        && let Some(fp)=fingerprint(&proxy["client-fingerprint"]) {
        if !tls["utls"].is_object() {tls["utls"]=json!({});}
        tls["utls"]["enabled"]=json!(true);tls["utls"]["fingerprint"]=json!(fp);
    }
    if proxy["_ech"].is_object() {tls["ech"]=proxy["_ech"].clone();}
    else if proxy["ech-opts"].is_object() {
        let raw=&proxy["ech-opts"];let mut ech=json!({});
        copy(&mut ech,raw,&[("enable","enabled")]);
        if (raw["config"].is_string()||raw["config"].is_array()) && let Some(value)=pem(&raw["config"]) {ech["config"]=value;}
        copy(&mut ech,raw,&[("query-server-name","query_server_name"),("config-path","config_path"),("fragment","fragment"),("fragment-fallback-delay","fragment_fallback_delay"),("record-fragment","record_fragment")]);
        tls["ech"]=ech;
    }
    if proxy["_curve_preferences"].is_array() {tls["curve_preferences"]=proxy["_curve_preferences"].clone();}
    if truthy(&proxy["_fragment"]){tls["fragment"]=json!(true);}
    if truthy(&proxy["_fragment_fallback_delay"]){tls["fragment_fallback_delay"]=proxy["_fragment_fallback_delay"].clone();}
    if truthy(&proxy["_record_fragment"]){tls["record_fragment"]=json!(true);}
    for (from,to) in [("_client_certificate","client_certificate"),("_client_certificate_path","client_certificate_path"),("_client_key","client_key"),("_client_key_path","client_key_path")] {if truthy(&proxy[from]) {tls[to]=proxy[from].clone();}}
    if !truthy(&tls["enabled"]) {out.as_object_mut().unwrap().shift_remove("tls");}
    else {certificate(proxy,tls)?;}
    Ok(())
}

fn csv(value: &Value) -> Value {
    if value.is_array() {value.clone()}else{json!(codec::js_string(value).split(',').map(codec::trim).collect::<Vec<_>>())}
}

fn ws_headers(raw: &Value) -> Result<Map<String,Value>,()> {
    if raw.is_null() {return Err(())}
    let mut out=Map::new();
    for (key,value) in fields(raw) {
        if value=="" {continue}
        let values=if value.is_array(){value}else{json!([codec::js_string(&value)])};
        if has_length(&values) {out.insert(key,values);}
    }
    let host=out.get("Host").ok_or(())?.as_array().ok_or(())?;
    if host.len()==1 {
        let host=codec::js_string(&host[0]);
        for item in format!("Host:{host}").split('\n') {
            let mut parts=item.split(':');let key=parts.next().unwrap();let value=parts.next().ok_or(())?;
            if codec::trim(value).is_empty() {continue}
            out.insert(codec::trim(key).to_owned(),json!(codec::trim(value).split(',').collect::<Vec<_>>()));
        }
    }
    Ok(out)
}

fn compact_headers(raw: &mut Value) {
    if let Some(headers)=raw.as_object_mut() {
        for value in headers.values_mut() {if value.as_array().is_some_and(|values|values.len()==1) {*value=value[0].clone();}}
    }
}

fn transport(proxy: &Value, out: &mut Value) -> Result<(),()> {
    let kind=proxy["network"].as_str().unwrap_or("");
    let mut result=match kind {
        "ws"=>json!({"type":"ws","headers":{}}),"http"=>json!({"type":"http","headers":{}}),"h2"=>json!({"type":"http"}),"grpc"=>json!({"type":"grpc"}),_=>return Ok(()),
    };
    match kind {
        "ws"=>{
            if truthy(&proxy["ws-opts"]) {
                let raw=&proxy["ws-opts"];
                copy(&mut result,raw,&[("early-data-header-name","early_data_header_name")]);
                if truthy(&raw["max-early-data"]) {result["max_early_data"]=json!(integer(raw.get("max-early-data")));}
                if let Some(path)=raw.get("path").filter(|value|**value!="") {result["path"]=json!(codec::js_string(path));}
                if let Some(headers)=raw.get("headers") {
                    if headers.is_null(){return Err(())}
                    if !fields(headers).is_empty() {result["headers"]=Value::Object(ws_headers(headers)?);}
                }
            }
            if truthy(&proxy["ws-headers"]) {let mut headers=fields(&result["headers"]);headers.extend(ws_headers(&proxy["ws-headers"])?);result["headers"]=Value::Object(headers);}
            if truthy(&proxy["ws-path"]) {result["path"]=json!(text(proxy,"ws-path"));}
            if truthy(&result["path"]) {
                let (path,ed)=crate::conversion::path_ed(result["path"].as_str().unwrap());
                if let Some(ed)=ed {result["path"]=json!(path);result["early_data_header_name"]=json!("Sec-WebSocket-Protocol");result["max_early_data"]=json!(integer(Some(&json!(ed))));}
            }
            if truthy(&proxy["ws-opts"]["v2ray-http-upgrade"]) {
                result["type"]=json!("httpupgrade");
                if truthy(&result["headers"]["Host"]) {result["host"]=result["headers"]["Host"][0].clone();result["headers"].as_object_mut().unwrap().shift_remove("Host");}
                for key in ["max_early_data","early_data_header_name"] {if truthy(&result[key]) {result.as_object_mut().unwrap().shift_remove(key);}}
            }
            compact_headers(&mut result["headers"]);
        },
        "http"=>{
            if truthy(&proxy["http-opts"]) {
                let raw=&proxy["http-opts"];if let Some(value)=raw.get("method").filter(|value|**value!="") {result["method"]=value.clone();}
                if let Some(value)=raw.get("path") {if let Some(values)=value.as_array() {result["path"]=json!(values.first().map(codec::js_string).unwrap_or_else(||"undefined".to_owned()));}else if *value!=""{result["path"]=json!(codec::js_string(value));}}
                if let Some(headers)=raw.get("headers") {
                    if headers.is_null(){return Err(())}
                    for (key,value) in fields(headers) {
                        if value==""{continue}let value=csv(&value);
                        if has_length(&value) {if key.eq_ignore_ascii_case("host"){result["host"]=value;}else{result["headers"][&key]=value;}}
                    }
                }
            }
            if truthy(&proxy["http-host"]) {let value=csv(&proxy["http-host"]);if has_length(&value){result["host"]=value;}}
            if truthy(&proxy["http-path"]) {let value=&proxy["http-path"];result["path"]=json!(if let Some(values)=value.as_array(){values.first().map(codec::js_string).unwrap_or_else(||"undefined".to_owned())}else{codec::js_string(value)});}
            if result["host"].as_array().is_some_and(|values|values.len()==1) {result["host"]=result["host"][0].clone();}
            compact_headers(&mut result["headers"]);
        },
        "h2"=>{
            if truthy(&proxy["h2-opts"]) {let raw=&proxy["h2-opts"];if let Some(value)=raw.get("path").filter(|value|**value!=""){result["path"]=json!(codec::js_string(value));}if let Some(value)=raw.get("host").filter(|value|**value!=""){let value=csv(value);if has_length(&value){result["host"]=value;}}}
            if truthy(&proxy["h2-host"]) {let value=csv(&proxy["h2-host"]);if has_length(&value){result["host"]=value;}}
            if truthy(&proxy["h2-path"]) {result["path"]=json!(text(proxy,"h2-path"));}
            out["tls"]["enabled"]=json!(true);
            let values=result["host"].as_array().ok_or(())?;if values.len()==1 {result["host"]=values[0].clone();}
        },
        "grpc"=>{if let Some(value)=proxy["grpc-opts"].get("grpc-service-name").filter(|value|!value.is_null()&&**value!=""){result["service_name"]=json!(codec::js_string(value));}},
        _=>{},
    }
    out["transport"]=result;Ok(())
}

fn plugin_mux(value: &Value) -> Value {
    if let Some(value)=value.as_bool(){return json!(u8::from(value))}
    if let Some(raw)=value.as_str() {let raw=codec::trim(raw).to_lowercase();match raw.as_str() {"true"=>return json!(1),"false"=>return json!(0),_=>{if !raw.is_empty()&&raw.bytes().all(|c|c.is_ascii_digit()){return json!(integer(Some(&json!(raw))))}}}}
    value.clone()
}

fn shadowsocks(proxy: &Value, shadow: Option<&Value>) -> Result<Vec<Value>,()> {
    if let Some(shadow)=shadow {
        let mut out=json!({});copy(&mut out,proxy,&[("name","tag")]);out["type"]=json!("shadowsocks");copy(&mut out,proxy,&[("cipher","method"),("password","password")]);out["detour"]=json!(format!("{}_shadowtls",text(proxy,"name")));udp_over_tcp(proxy,&mut out);network(proxy,&mut out);smux(proxy,&mut out);
        return Ok(vec![out,shadow_outbound(proxy,shadow)?])
    }
    let mut out=base(proxy,"shadowsocks")?;copy(&mut out,proxy,&[("cipher","method"),("password","password")]);udp_over_tcp(proxy,&mut out);fragment(proxy,&mut out);network(proxy,&mut out);tfo(proxy,&mut out);detour(proxy,&mut out);smux(proxy,&mut out);resolver(proxy,&mut out);
    if truthy(&proxy["plugin"]) {
        let mut options=Vec::new();
        if proxy["plugin"].as_str().is_some_and(|value|["obfs","v2ray-plugin"].contains(&value)) {
            if proxy["plugin-opts"].is_null(){return Err(())}
            let mut raw=fields(&proxy["plugin-opts"]);
            if proxy["plugin"]=="obfs" {
                out["plugin"]=json!("obfs-local");out["plugin_opts"]=json!("");
                if truthy(&proxy["obfs-host"]){raw.insert("host".to_owned(),proxy["obfs-host"].clone());}
                for (key,value) in fields(&Value::Object(raw)) {let label=match key.as_str() {"mode"=>"obfs","host"=>"obfs-host",_=>&key};options.push(format!("{label}={}",codec::js_string(&value)));}
            }else{
                out["plugin"]=json!("v2ray-plugin");
                for (from,to) in [("ws-host","host"),("ws-path","path")] {if truthy(&proxy[from]){raw.insert(to.to_owned(),proxy[from].clone());}}
                for (key,value) in fields(&Value::Object(raw)) {match key.as_str() {
                    "tls"=>{if truthy(&value){options.push("tls".to_owned());}},
                    "headers"=>options.push(format!("headers={}",codec::stringify(&value))),
                    "mux"=>{let mux=plugin_mux(&value);if truthy(&mux){out["multiplex"]=json!({"enabled":true});}options.push(format!("mux={}",codec::js_string(&mux)));},
                    _=>options.push(format!("{key}={}",codec::js_string(&value))),
                }}
            }
        }
        out["plugin_opts"]=json!(options.join(";"));
    }
    Ok(vec![out])
}

fn ssr(proxy: &Value) -> Result<Value,()> {
    let mut out=base(proxy,"shadowsocksr")?;
    copy(&mut out,proxy,&[("cipher","method"),("password","password"),("obfs","obfs"),("protocol","protocol")]);
    for (from,to) in [("obfs-param","obfs_param"),("protocol-param","protocol_param")] {if truthy(&proxy[from]){out[to]=proxy[from].clone();}}
    fragment(proxy,&mut out);network(proxy,&mut out);tfo(proxy,&mut out);detour(proxy,&mut out);smux(proxy,&mut out);resolver(proxy,&mut out);Ok(out)
}

fn shadow_opts(proxy: &Value) -> Option<Value> {
    if proxy["plugin"]=="shadow-tls"&&truthy(&proxy["plugin-opts"]){return Some(proxy["plugin-opts"].clone())}
    if proxy["type"]=="snell"&&proxy["obfs-opts"]["mode"]=="shadow-tls" {
        let mut out=json!({});copy(&mut out,&proxy["obfs-opts"],&[("host","host"),("password","password"),("version","version"),("alpn","alpn")]);return Some(out)
    }
    None
}

fn alpn(value: &Value) -> Option<Value> {
    match value {
        Value::String(value)=>Some(json!(value.split(',').map(codec::trim).filter(|value|!value.is_empty()).collect::<Vec<_>>())),
        Value::Array(_)=>Some(value.clone()),_=>None,
    }
}

fn shadow_outbound(proxy: &Value, options: &Value) -> Result<Value,()> {
    let mut out=json!({"tag":format!("{}_shadowtls",text(proxy,"name")),"type":"shadowtls"});copy(&mut out,proxy,&[("server","server")]);
    let port=integer(proxy.get("port"));if port<0.0||port>65535.0 {return Err(())}
    out["server_port"]=json!(port);copy(&mut out,options,&[("version","version"),("password","password")]);
    out["tls"]=json!({"enabled":true});copy(&mut out["tls"],options,&[("host","server_name")]);
    if truthy(&proxy["skip-cert-verify"]){out["tls"]["insecure"]=json!(true);}
    if let Some(value)=fingerprint(&proxy["client-fingerprint"]){out["tls"]["utls"]=json!({"enabled":true,"fingerprint":value});}
    if let Some(value)=alpn(&options["alpn"]).or_else(||alpn(&proxy["alpn"])){out["tls"]["alpn"]=value;}
    certificate(proxy,&mut out["tls"])?;
    if proxy["fast-open"]==true {out["udp_fragment"]=json!(true);}
    tfo(proxy,&mut out);detour(proxy,&mut out);resolver(proxy,&mut out);Ok(out)
}

fn snell(proxy: &Value, include: bool, shadow: Option<&Value>) -> Result<Vec<Value>,()> {
    let version=if let Some(value)=proxy.get("version").filter(|value|!value.is_null()) {
        let raw=codec::js_string(value);let raw=codec::trim(&raw);
        if raw.is_empty()||!raw.bytes().all(|c|c.is_ascii_digit()){return Err(())}
        let version=integer(Some(&json!(raw)));
        let allowed:&[f64]=if include {&[1.0,2.0,3.0,4.0,5.0,6.0]}else{&[4.0,5.0,6.0]};
        if !allowed.contains(&version){return Err(())}
        Some(version)
    }else{None};
    let output_version=version.map(|version|if !include&&version==5.0{4.0}else{version});
    let mut out=base(proxy,"snell")?;copy(&mut out,proxy,&[("psk","psk")]);if let Some(value)=output_version {out["version"]=json!(value);}
    if truthy(&proxy["_userkey"]){out["userkey"]=proxy["_userkey"].clone();}
    if output_version==Some(6.0) {
        if truthy(&proxy["mode"]){out["mode"]=proxy["mode"].clone();}
        if include&&truthy(&proxy["quic-proxy-mode"]){out["quic_proxy_mode"]=json!(true);}
    }else if proxy["obfs-opts"]["mode"]!="shadow-tls" {
        for (from,to) in [("mode","obfs_mode"),("host","obfs_host")] {if truthy(&proxy["obfs-opts"][from]){out[to]=proxy["obfs-opts"][from].clone();}}
    }
    if truthy(&proxy["reuse"])&&version.is_none_or(|value|value>=4.0){out["reuse"]=json!(true);}
    network(proxy,&mut out);
    if let Some(shadow)=shadow {
        out["detour"]=json!(format!("{}_shadowtls",text(proxy,"name")));out.as_object_mut().unwrap().shift_remove("server");out.as_object_mut().unwrap().shift_remove("server_port");
        Ok(vec![out,shadow_outbound(proxy,shadow)?])
    }else{fragment(proxy,&mut out);tfo(proxy,&mut out);detour(proxy,&mut out);resolver(proxy,&mut out);Ok(vec![out])}
}

fn packet_encoding(proxy: &Value, out: &mut Value) {
    if let Some(value)=proxy.get("packet-encoding").filter(|value|!value.is_null()) {
        let value=codec::trim(&codec::js_string(value)).to_lowercase();if ["","packetaddr","xudp"].contains(&value.as_str()){out["packet_encoding"]=json!(value);}
    }else if truthy(&proxy["xudp"]){out["packet_encoding"]=json!("xudp");}
    else if truthy(&proxy["packet-addr"]){out["packet_encoding"]=json!("packetaddr");}
}

fn stream(proxy: &Value) -> Result<Value,()> {
    let kind=text(proxy,"type");let mut out=base(proxy,&kind)?;
    if kind=="trojan" {copy(&mut out,proxy,&[("password","password")]);}
    else {copy(&mut out,proxy,&[("uuid","uuid")]);if kind=="vmess" {out["security"]=vmess_security(&proxy["cipher"]);out["alter_id"]=json!(integer(proxy.get("alterId")));}}
    initial_tls(proxy,&mut out,kind=="trojan");
    if kind!="trojan" {
        packet_encoding(proxy,&mut out);
        if kind=="vmess" {
            for (from,to) in [("global-padding","global_padding"),("authenticated-length","authenticated_length")] {if let Some(value)=proxy.get(from).filter(|value|!value.is_null()){out[to]=json!(truthy(value));}}
        }
    }
    fragment(proxy,&mut out);
    if kind=="vless" {if let Some(value)=proxy.get("flow").filter(|value|!value.is_null()){out["flow"]=value.clone();}}
    if kind!="trojan"||["ws","grpc"].contains(&text(proxy,"network").as_str()){transport(proxy,&mut out)?;}
    network(proxy,&mut out);tfo(proxy,&mut out);detour(proxy,&mut out);
    if kind=="vless" {smux(proxy,&mut out);tls(proxy,&mut out)?;}else{tls(proxy,&mut out)?;smux(proxy,&mut out);}
    resolver(proxy,&mut out);Ok(out)
}

fn ssh(proxy: &Value) -> Result<Value,()> {
    let mut out=base(proxy,"ssh")?;
    for (from,to) in [("username","user"),("password","password")] {if truthy(&proxy[from]){out[to]=proxy[from].clone();}}
    let key=if truthy(&proxy["private-key"]){&proxy["private-key"]}else{&proxy["privateKey"]};
    if truthy(key) {
        let inline=match key {Value::String(value)=>value.contains("PRIVATE KEY"),Value::Array(values)=>values.contains(&json!("PRIVATE KEY")),_=>return Err(())};
        out[if inline{"private_key"}else{"private_key_path"}]=key.clone();
    }
    if truthy(&proxy["private-key-passphrase"]){out["private_key_passphrase"]=proxy["private-key-passphrase"].clone();}
    if truthy(&proxy["server-fingerprint"]) {
        let fingerprint=proxy["server-fingerprint"].as_str().ok_or(())?;out["host_key"]=json!([fingerprint]);out["host_key_algorithms"]=json!([fingerprint.split(' ').next().unwrap()]);
    }
    for (from,to) in [("host-key","host_key"),("host-key-algorithms","host_key_algorithms")] {if truthy(&proxy[from]){out[to]=proxy[from].clone();}}
    fragment(proxy,&mut out);tfo(proxy,&mut out);detour(proxy,&mut out);resolver(proxy,&mut out);Ok(out)
}

fn http_socks_naive(proxy: &Value) -> Result<Value,()> {
    let kind=text(proxy,"type");let socks=kind=="socks5";let naive=kind=="naive";
    let mut out=base(proxy,if socks{"socks"}else{&kind})?;
    if socks {out["version"]=json!("5");}else{initial_tls(proxy,&mut out,naive);}
    for key in ["username","password"] {if truthy(&proxy[key]){out[key]=proxy[key].clone();}}
    if kind=="http"&&truthy(&proxy["headers"]) {let headers=fields(&proxy["headers"]).into_iter().map(|(key,value)|(key,json!(codec::js_string(&value)))).collect::<Map<_,_>>();if !headers.is_empty(){out["headers"]=Value::Object(headers);}}
    if socks||naive {udp_over_tcp(proxy,&mut out);}
    if naive {
        let concurrency=integer(proxy.get("insecure-concurrency"));if concurrency.is_finite()&&concurrency>=0.0{out["insecure_concurrency"]=json!(concurrency);}
        if truthy(&proxy["extra-headers"]){out["extra_headers"]=proxy["extra-headers"].clone();}
        if truthy(&proxy["quic"]){out["quic"]=json!(true);}
        if truthy(&proxy["quic-congestion-control"]){out["quic_congestion_control"]=proxy["quic-congestion-control"].clone();}
    }
    fragment(proxy,&mut out);if socks {network(proxy,&mut out);}tfo(proxy,&mut out);detour(proxy,&mut out);
    if !socks {tls(proxy,&mut out)?;}
    if naive {smux(proxy,&mut out);}
    resolver(proxy,&mut out);
    if naive&&truthy(&out["tls"]["insecure"]) {out["tls"].as_object_mut().unwrap().shift_remove("insecure");}
    Ok(out)
}

fn trim_delimiter_space(value: &str, delimiter: char) -> Vec<&str> {
    let values:Vec<_>=value.split(delimiter).collect();
    let count=values.len();
    values.into_iter().enumerate().map(|(index,value)|{
        let value=if index>0{value.trim_start_matches(codec::is_space)}else{value};
        if index+1<count{value.trim_end_matches(codec::is_space)}else{value}
    }).collect()
}

fn bitrate(value: &str) -> bool {
    let Some(value)=value.strip_suffix("ps").and_then(|value|value.strip_suffix('B').or_else(||value.strip_suffix('b')))else{return false};
    let digits=value.bytes().take_while(u8::is_ascii_digit).count();
    digits>0&&value[digits..].trim_start_matches([' ','\t']).bytes().all(|c|b"KMGT".contains(&c))
}

fn hopping(proxy: &Value, out: &mut Value) -> Result<(),()> {
    if truthy(&proxy["hop-interval"]) {let value=&proxy["hop-interval"];out["hop_interval"]=if digits(Some(value)){json!(format!("{}s",codec::js_string(value)))}else{value.clone()};}
    if truthy(&proxy["ports"]) {
        let raw=proxy["ports"].as_str().ok_or(())?;
        let ports=trim_delimiter_space(raw,',');
        out["server_ports"]=json!(ports.into_iter().map(|value| {let value=trim_delimiter_space(value,'-').join(":");if value.contains(':'){value}else{format!("{value}:{value}")}}).collect::<Vec<_>>());
    }
    Ok(())
}

fn hysteria(proxy: &Value) -> Result<Value,()> {
    let mut out=base(proxy,"hysteria")?;out["disable_mtu_discovery"]=json!(false);initial_tls(proxy,&mut out,true);hopping(proxy,&mut out)?;
    for key in ["auth_str","auth-str"] {if truthy(&proxy[key]){out["auth_str"]=json!(text(proxy,key));}}
    fragment(proxy,&mut out);
    for (key,mbps) in [("up","up_mbps"),("down","down_mbps")] {
        let value=text(proxy,key);if bitrate(&value)&&!value.ends_with("Mbps"){out[key]=json!(value);}else{out[mbps]=json!(integer(proxy.get(key)));}
    }
    for (from,to) in [("obfs","obfs"),("recv_window_conn","recv_window_conn"),("recv-window-conn","recv_window_conn"),("recv_window","recv_window"),("recv-window","recv_window")] {if truthy(&proxy[from]){out[to]=proxy[from].clone();}}
    if proxy["disable_mtu_discovery"]==true||proxy["disable_mtu_discovery"]==1{out["disable_mtu_discovery"]=json!(true);}
    network(proxy,&mut out);tls(proxy,&mut out)?;detour(proxy,&mut out);tfo(proxy,&mut out);smux(proxy,&mut out);resolver(proxy,&mut out);Ok(out)
}

fn hysteria2(proxy: &Value) -> Result<Value,()> {
    let mut out=base(proxy,"hysteria2")?;copy(&mut out,proxy,&[("password","password")]);out["obfs"]=json!({});initial_tls(proxy,&mut out,true);hopping(proxy,&mut out)?;
    for (from,to) in [("up","up_mbps"),("down","down_mbps")] {if truthy(&proxy[from]){out[to]=json!(integer(proxy.get(from)));}}
    if proxy["obfs"].as_str().is_some_and(|value|["salamander","gecko"].contains(&value)){out["obfs"]["type"]=proxy["obfs"].clone();}
    if proxy["obfs"]=="gecko" {
        let has=|key|proxy.get(key).is_some_and(|value|!value.is_null()&&codec::js_string(value)!="");
        let has_min=has("obfs-min-packet-size");let has_max=has("obfs-max-packet-size");
        if has_min||has_max {
            let parse=|key|proxy.get(key).and_then(|value|safe_integer(Some(&json!(codec::trim(&codec::js_string(value))))));
            let min=parse("obfs-min-packet-size");let raw_max=parse("obfs-max-packet-size");let max=raw_max.map(|value|value.min(2048));
            if !(has_min&&min.is_none_or(|value|value==0)||has_max&&raw_max.is_none_or(|value|value==0)||max.unwrap_or(1200)<min.unwrap_or(512)) {
                if has_min{out["obfs"]["min_packet_size"]=json!(min);}
                if has_max{out["obfs"]["max_packet_size"]=json!(max);}
            }
        }
    }
    if truthy(&proxy["obfs-password"]){out["obfs"]["password"]=proxy["obfs-password"].clone();}
    if !truthy(&out["obfs"]["type"]){out.as_object_mut().unwrap().shift_remove("obfs");}
    if truthy(&proxy["bbr-profile"]){out["bbr_profile"]=proxy["bbr-profile"].clone();}
    if truthy(&proxy["disable-chrome-parrot"]){out["disable_chrome_parrot"]=json!(true);}
    network(proxy,&mut out);tls(proxy,&mut out)?;tfo(proxy,&mut out);detour(proxy,&mut out);smux(proxy,&mut out);resolver(proxy,&mut out);Ok(out)
}

fn tuic(proxy: &Value) -> Result<Value,()> {
    let mut out=base(proxy,"tuic")?;copy(&mut out,proxy,&[("uuid","uuid"),("password","password")]);initial_tls(proxy,&mut out,true);fragment(proxy,&mut out);
    for (from,to,default) in [("congestion-controller","congestion_control","cubic"),("udp-relay-mode","udp_relay_mode","native")] {if truthy(&proxy[from])&&proxy[from]!=default{out[to]=proxy[from].clone();}}
    if truthy(&proxy["reduce-rtt"]){out["zero_rtt_handshake"]=json!(true);}
    if truthy(&proxy["udp-over-stream"]){out["udp_over_stream"]=json!(true);}
    if truthy(&proxy["heartbeat-interval"]){out["heartbeat"]=json!(format!("{}ms",text(proxy,"heartbeat-interval")));}
    network(proxy,&mut out);tfo(proxy,&mut out);detour(proxy,&mut out);tls(proxy,&mut out)?;smux(proxy,&mut out);resolver(proxy,&mut out);Ok(out)
}

fn anytls(proxy: &Value) -> Result<Value,()> {
    let mut out=base(proxy,"anytls")?;copy(&mut out,proxy,&[("password","password")]);initial_tls(proxy,&mut out,true);
    if truthy(&proxy["client-metadata"]){out["client_metadata"]=json!(text(proxy,"client-metadata"));}
    for (from,to) in [("idle-session-check-interval","idle_session_check_interval"),("idle-session-timeout","idle_session_timeout")] {if digits(proxy.get(from)){out[to]=json!(format!("{}s",text(proxy,from)));}}
    if digits(proxy.get("min-idle-session")){out["min_idle_session"]=json!(integer(proxy.get("min-idle-session")));}
    if let Some(value)=proxy.get("disable-reuse").filter(|value|!value.is_null()){out["disable_reuse"]=json!(truthy(value));}
    detour(proxy,&mut out);tls(proxy,&mut out)?;resolver(proxy,&mut out);Ok(out)
}

fn tailscale(proxy: &Value) -> Value {
    let raw=&proxy["control-http-client"];
    let control=match raw {Value::Null=>false,Value::String(value)=>!codec::trim(value).is_empty(),Value::Object(fields)=>fields.values().any(|value|!value.is_null()&&*value!=""),_=>true};
    let mut out=json!({});copy(&mut out,proxy,&[("name","tag")]);out["type"]=json!("tailscale");
    if let Some(port)=safe_integer(proxy.get("_listen_port")).filter(|port|*port<=65535){out["listen_port"]=json!(port);}
    if proxy["_taildrop_directory"].is_string(){out["taildrop_directory"]=proxy["_taildrop_directory"].clone();}
    if proxy["_on_demand"].is_boolean(){out["on_demand"]=proxy["_on_demand"].clone();}
    copy(&mut out,proxy,&[("control-http-client","control_http_client"),("udp-timeout","udp_timeout")]);
    if truthy(&proxy["state-dir"]){out["state_directory"]=proxy["state-dir"].clone();}else{copy(&mut out,proxy,&[("state-directory","state_directory")]);}
    copy(&mut out,proxy,&[("auth-key","auth_key"),("control-url","control_url"),("ephemeral","ephemeral"),("hostname","hostname"),("accept-routes","accept_routes"),("exit-node","exit_node"),("exit-node-allow-lan-access","exit_node_allow_lan_access")]);
    if proxy["advertise-routes"].is_array(){out["advertise_routes"]=proxy["advertise-routes"].clone();}
    copy(&mut out,proxy,&[("advertise-exit-node","advertise_exit_node")]);
    for (from,to) in [("advertise-tags","advertise_tags"),("relay-server-static-endpoints","relay_server_static_endpoints")] {if proxy[from].is_array(){out[to]=proxy[from].clone();}}
    copy(&mut out,proxy,&[("system-interface","system_interface"),("system-interface-name","system_interface_name")]);
    for (from,to) in [("system-interface-mtu","system_interface_mtu"),("relay-server-port","relay_server_port")] {if digits(proxy.get(from)){out[to]=json!(integer(proxy.get(from)));}}
    if !control{detour(proxy,&mut out);resolver(proxy,&mut out);}
    if proxy["ssh-server"].is_object() {
        let mut ssh=json!({"enabled":proxy["ssh-server"]["enabled"]!=false});copy(&mut ssh,&proxy["ssh-server"],&[("disable-pty","disable_pty"),("disable-sftp","disable_sftp"),("disable-forwarding","disable_forwarding")]);out["ssh_server"]=ssh;
    }else if truthy(&proxy["ssh-server"]){out["ssh_server"]=json!(true);}
    out
}

fn wireguard(proxy: &Value) -> Result<Value,()> {
    let address:Vec<Value>=["ipv4","ipv6"].iter().filter_map(|family|wireguard_address(proxy,family)).collect();
    let mut out=json!({"system":truthy(&proxy["system"])});
    if proxy["_name"].is_string(){out["name"]=proxy["_name"].clone();}
    if let Some(port)=safe_integer(proxy.get("_listen_port")).filter(|port|*port<=65535){out["listen_port"]=json!(port);}
    if proxy["_on_demand"].is_boolean(){out["on_demand"]=proxy["_on_demand"].clone();}
    if truthy(&proxy["mtu"]){out["mtu"]=json!(integer(proxy.get("mtu")));}
    copy(&mut out,proxy,&[("udp-timeout","udp_timeout")]);
    if let Some(max)=safe_integer(proxy.get("_udp_nat_max")).filter(|max|*max<=4294967295){out["udp_nat_max"]=json!(max);}
    if truthy(&proxy["workers"]){out["workers"]=json!(integer(proxy.get("workers")));}
    copy(&mut out,proxy,&[("name","tag")]);out["type"]=json!("wireguard");
    out["address"]=json!(address);copy(&mut out,proxy,&[("private-key","private_key")]);
    for field in ["udp_mapping","udp_filtering"] {let value=&proxy[format!("_{field}")];if value.as_str().is_some_and(|value|["endpoint_independent","address_dependent","address_and_port_dependent"].contains(&value)){out[field]=value.clone();}}
    let root_port=integer(proxy.get("port"));if root_port<0.0||root_port>65535.0{return Err(())}
    fragment(proxy,&mut out);
    let root_reserved=proxy.get("reserved").filter(|value|value.is_string()||value.is_array());
    let default_peer=vec![json!({})];
    let peers=proxy["peers"].as_array().filter(|values|!values.is_empty()).unwrap_or(&default_peer);
    let mut output_peers=Vec::new();
    for peer in peers {
        if peer.is_null(){return Err(())}
        let mut value=json!({});
        let server=if truthy(&peer["server"])&&truthy(&peer["port"]){peer.get("server")}else{proxy.get("server")};
        if let Some(server)=server{value["address"]=server.clone();}
        value["port"]=json!(if truthy(&peer["server"])&&truthy(&peer["port"]){integer(peer.get("port"))}else{root_port});
        if truthy(&peer["persistent-keepalive-interval"]){value["persistent_keepalive_interval"]=json!(integer(peer.get("persistent-keepalive-interval")));}
        for (keys,root,to) in [(["public-key","public_key"],"public-key","public_key"),(["pre-shared-key","pre_shared_key"],"pre-shared-key","pre_shared_key")] {
            if let Some(raw)=keys.iter().find_map(|key|peer.get(key).filter(|value|truthy(value))).or_else(||proxy.get(root)){value[to]=raw.clone();}
        }
        let allowed=peer.get("allowed-ips").filter(|value|truthy(value)).or_else(||peer.get("allowed_ips").filter(|value|truthy(value))).cloned().unwrap_or_else(||if truthy(&proxy["ipv6"]){json!(["0.0.0.0/0","::/0"])}else{json!(["0.0.0.0/0"])});
        value["allowed_ips"]=allowed;
        let reserved=match &peer["reserved"] {
            Value::String(raw)=>Some(json!([raw])),Value::Array(values) if !values.is_empty()=>Some(json!(values)),_=>root_reserved.cloned(),
        };
        if let Some(raw)=reserved{value["reserved"]=raw;}
        output_peers.push(value);
    }
    out["peers"]=json!(output_peers);tfo(proxy,&mut out);detour(proxy,&mut out);smux(proxy,&mut out);resolver(proxy,&mut out);Ok(out)
}

fn convert(proxy: &Value, include: bool) -> Result<Vec<Value>,()> {
    let options=shadow_opts(proxy);
    let shadow_enabled=options.as_ref().is_some_and(|options|truthy(&options["password"])||options.get("version").is_some_and(|value|!value.is_null()&&codec::to_number(value)!=0.0));
    let kind=text(proxy,"type");
    let mut stream_shadow=None;
    if shadow_enabled&&["vmess","vless","trojan"].contains(&kind.as_str()) {
        if truthy(&proxy["reality-opts"])||(["vmess","vless"].contains(&kind.as_str())&&proxy["network"]=="h2")||(kind=="vless"&&proxy["flow"]=="xtls-rprx-vision"){return Err(())}
        let mut options=options.clone().unwrap();
        let raw=&options["version"];
        let parsed=if raw.as_str().is_some_and(|value|codec::trim(value).is_empty()){f64::NAN}else{codec::to_number(raw)};
        let version=if parsed==0.0{2.0}else{parsed};if ![1.0,2.0,3.0].contains(&version){return Err(())}
        if !options.is_object(){options=Value::Object(fields(&options));}
        options["version"]=json!(version);stream_shadow=Some(shadow_outbound(proxy,&options)?);
    }
    if (kind=="anytls"&&shadow_enabled)||proxy["network"]=="xhttp"{return Err(())}
    let mut list=match kind.as_str() {
        "ssh"=>vec![ssh(proxy)?],
        "http"|"socks5"|"naive"=>{if kind=="socks5"&&truthy(&proxy["tls"]){return Err(())}vec![http_socks_naive(proxy)?]},
        "ss"=>shadowsocks(proxy,if proxy["plugin"]=="shadow-tls"{Some(options.as_ref().ok_or(())?)}else{None})?,
        "ssr" if include=>vec![ssr(proxy)?],
        "snell"=>snell(proxy,include,options.as_ref())?,
        "vmess" if !truthy(&proxy["network"])||proxy["network"].as_str().is_some_and(|value|["tcp","ws","grpc","h2","http"].contains(&value))=>vec![stream(proxy)?],
        "vless" if (!truthy(&proxy["encryption"])||proxy["encryption"]=="none")&&(!truthy(&proxy["flow"])||proxy["flow"]=="xtls-rprx-vision")=>vec![stream(proxy)?],
        "trojan" if !truthy(&proxy["flow"])=>vec![stream(proxy)?],
        "hysteria"=>vec![hysteria(proxy)?],"hysteria2"=>vec![hysteria2(proxy)?],
        "tuic" if !truthy(&proxy["token"])||proxy["token"].as_array().is_some_and(Vec::is_empty)||proxy["token"]["length"]==0=>vec![tuic(proxy)?],
        "wireguard"=>vec![wireguard(proxy)?],"anytls"=>vec![anytls(proxy)?],"tailscale"=>vec![tailscale(proxy)],_=>return Err(()),
    };
    if let Some(shadow)=stream_shadow {list[0]["detour"]=json!(format!("{}_shadowtls",text(proxy,"name")));list[0].as_object_mut().unwrap().shift_remove("tls");list.push(shadow);}
    if include&&truthy(&proxy["name-cert-verify"]) {for proxy_out in &mut list{if truthy(&proxy_out["tls"]){proxy_out["tls"]["certificate_server_name"]=proxy["name-cert-verify"].clone();}}}
    Ok(list)
}

pub fn produce(proxies: Vec<Value>, kind: &str, opts: &Value) -> Result<Value,Error> {
    let originals=proxies.clone();
    let proxies=crate::clash_output::produce(proxies,"internal",&json!({"include-unsupported-proxy":true}),crate::clash_output::Family::Meta)?;
    let mut list=Vec::new();
    for (mut proxy,original) in proxies.as_array().unwrap().iter().cloned().zip(originals) {
        if original["plugin"]=="shadow-tls"&&truthy(&original["plugin-opts"]) {
            proxy["plugin"]=original["plugin"].clone();proxy["plugin-opts"]=original["plugin-opts"].clone();
            if truthy(&original["obfs-opts"]){proxy["obfs-opts"]=original["obfs-opts"].clone();}else{proxy.as_object_mut().unwrap().shift_remove("obfs-opts");}
        }
        if let Ok(produced)=convert(&proxy,truthy(&opts["include-unsupported-proxy"])){list.extend(produced);}
    }
    if kind=="internal"{return Ok(json!(list))}
    let mut outbounds=Vec::new();let mut endpoints=Vec::new();
    for proxy in list {if ["wireguard","tailscale"].contains(&text(&proxy,"type").as_str()){endpoints.push(proxy);}else{outbounds.push(proxy);}}
    Ok(json!(String::from_utf8(codec::pretty(&json!({"outbounds":outbounds,"endpoints":endpoints}))).unwrap()))
}
