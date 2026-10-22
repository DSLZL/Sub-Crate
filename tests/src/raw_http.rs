use crate::{Result,json,require};
use base64::Engine;
use serde_json::{Value,json};
use std::{collections::BTreeMap,io::{Read,Write},net::TcpStream,time::Duration};

pub struct Reply {pub status:u16,pub headers:BTreeMap<String,String>,pub body:Vec<u8>}
impl Reply {pub fn value(&self)->Result<Value> {Ok(json!({"status":self.status,"headers":self.headers,"body":std::str::from_utf8(&self.body)?}))}pub fn evidence(&self)->Value {json!({"status":self.status,"headers":self.headers,"body_base64":base64::engine::general_purpose::STANDARD.encode(&self.body)})}}
pub fn request(base:&str,method:&str,path:&str,headers:&BTreeMap<String,String>,body:&[u8],chunk_size:Option<usize>)->Result<Reply> {
    let url=reqwest::Url::parse(base)?;require(url.scheme()=="http"&&url.username().is_empty()&&url.password().is_none()&&url.query().is_none()&&url.fragment().is_none(),"Raw fixtures require an HTTP origin")?;
    let host=url.host_str().ok_or("Missing raw HTTP host")?;let port=url.port_or_known_default().ok_or("Missing raw HTTP port")?;let mut socket=TcpStream::connect((host,port))?;socket.set_read_timeout(Some(Duration::from_secs(30)))?;socket.set_write_timeout(Some(Duration::from_secs(5)))?;
    require(!method.contains(['\r','\n',' '])&&!path.contains(['\r','\n']),"Invalid raw request line")?;let mut fields=headers.clone();fields.entry("Host".into()).or_insert_with(||format!("{host}:{port}"));fields.entry("Connection".into()).or_insert_with(||"close".into());
    if let Some(size)=chunk_size {require(size>0,"Chunk size must be positive")?;fields.insert("Transfer-Encoding".into(),"chunked".into());} else if !fields.keys().any(|key|key.eq_ignore_ascii_case("content-length")||key.eq_ignore_ascii_case("transfer-encoding")) {fields.insert("Content-Length".into(),body.len().to_string());}
    let mut wire=format!("{method}{}{path} HTTP/1.1\r\n",url.path().trim_end_matches('/')).into_bytes();for (key,value) in fields {require(!key.contains(['\r','\n',':'])&&!value.contains(['\r','\n']),"Invalid raw request header")?;wire.extend(format!("{key}: {value}\r\n").chars().map(|character|u8::try_from(character as u32).map_err(|_|"Raw HTTP header is not Latin1")).collect::<std::result::Result<Vec<_>,_>>()?);}wire.extend(b"\r\n");
    if let Some(size)=chunk_size {for chunk in body.chunks(size) {write!(wire,"{:x}\r\n",chunk.len())?;wire.extend(chunk);wire.extend(b"\r\n");}wire.extend(b"0\r\n\r\n");} else {wire.extend(body);}
    // One write also permits early CORS rejection before a large body on Windows.
    socket.write_all(&wire)?;let mut wire=Vec::new();socket.take(128*1024*1024).read_to_end(&mut wire)?;decode(&wire,method=="HEAD")
}
fn decode(wire:&[u8],head:bool)->Result<Reply> {
    let end=wire.windows(4).position(|bytes|bytes==b"\r\n\r\n").ok_or("Incomplete raw HTTP headers")?;let header_text=wire[..end].iter().map(|byte|char::from(*byte)).collect::<String>();let mut lines=header_text.split("\r\n");let first=lines.next().ok_or("Missing raw status")?;require(first.starts_with("HTTP/1."),"Invalid HTTP response version")?;let status=first.split_whitespace().nth(1).ok_or("Missing status code")?.parse()?;let mut headers=BTreeMap::new();for line in lines {let (key,value)=line.split_once(':').ok_or("Malformed response header")?;let key=key.to_lowercase();let value=value.trim().to_owned();if let Some(previous)=headers.get_mut(&key) {let previous:&mut String=previous;previous.push_str(", ");previous.push_str(&value);} else {headers.insert(key,value);}}
    let raw=&wire[end+4..];let mut body=Vec::new();if !head&&status!=304&&status!=204 {if headers.get("transfer-encoding").is_some_and(|value|value.eq_ignore_ascii_case("chunked")) {let mut rest=raw;loop {let line=rest.windows(2).position(|bytes|bytes==b"\r\n").ok_or("Missing chunk size")?;let length=usize::from_str_radix(std::str::from_utf8(&rest[..line])?.split(';').next().ok_or("Missing chunk length")?,16)?;rest=&rest[line+2..];if length==0 {require(rest.starts_with(b"\r\n")||rest.windows(4).any(|bytes|bytes==b"\r\n\r\n"),"Incomplete chunk trailers")?;break;}let chunk=rest.get(..length).ok_or("Incomplete response chunk")?;body.extend(chunk);require(rest.get(length..length+2)==Some(b"\r\n"),"Missing chunk terminator")?;rest=&rest[length+2..];}} else {body.extend(raw);}if let Some(length)=headers.get("content-length") {require(body.len()==length.parse::<usize>()?,"Raw response length mismatch")?;}}else {require(raw.is_empty(),"Suppressed response unexpectedly includes a body")?;}
    Ok(Reply{status,headers,body})
}
pub fn json_bytes(value:&Value)->Vec<u8> {json::stringify(value).into_bytes()}

#[cfg(test)]mod tests {use super::*;
    #[test]fn raw_decoder_preserves_chunks_and_rejects_truncation()->Result {let reply=decode(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n\0A\r\n1\r\n\xff\r\n0\r\n\r\n",false)?;require(reply.body==b"\0A\xff","Raw binary chunks changed")?;for bad in [b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nabc".as_slice(),b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nabc".as_slice()] {require(decode(bad,false).is_err(),"Truncated response accepted")?;}require(decode(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n",true)?.body.is_empty(),"HEAD body not suppressed")}
}
