use crate::{Result, actions_only, require};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::Duration;

pub struct Request {
    pub method: String,
    pub target: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub record: Arc<Mutex<Value>>,
}

impl Request {
    fn read(stream: &mut TcpStream) -> Result<Option<Self>> {
        let mut input = BufReader::new(stream);
        let mut first = String::new();
        if input.read_line(&mut first)? == 0 {
            return Ok(None);
        }
        let mut parts = first.split_whitespace();
        let method = parts.next().ok_or("Missing HTTP method")?.to_owned();
        let target = parts.next().ok_or("Missing HTTP target")?.to_owned();
        require(
            parts
                .next()
                .is_some_and(|version| version.starts_with("HTTP/1.")),
            "Invalid HTTP version",
        )?;
        let mut headers = BTreeMap::new();
        let mut total = first.len();
        loop {
            let mut line = String::new();
            require(input.read_line(&mut line)? > 0, "Incomplete HTTP headers")?;
            total += line.len();
            require(total <= 1024 * 1024, "Fixture header limit")?;
            if line == "\r\n" || line == "\n" {
                break;
            }
            let (key, value) = line.split_once(':').ok_or("Invalid HTTP header")?;
            headers.insert(key.to_lowercase(), value.trim().to_owned());
        }
        let mut body = Vec::new();
        if headers
            .get("transfer-encoding")
            .is_some_and(|value| value.eq_ignore_ascii_case("chunked"))
        {
            loop {
                let mut line = String::new();
                input.read_line(&mut line)?;
                let count = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16)?;
                require(body.len() + count <= 64 * 1024 * 1024, "Fixture body limit")?;
                if count == 0 {
                    loop {
                        line.clear();
                        require(
                            input.read_line(&mut line)? > 0,
                            "Missing chunk trailer terminator",
                        )?;
                        if line == "\r\n" {
                            break;
                        }
                    }
                    break;
                }
                let start = body.len();
                body.resize(start + count, 0);
                input.read_exact(&mut body[start..])?;
                let mut crlf = [0; 2];
                input.read_exact(&mut crlf)?;
                require(crlf == *b"\r\n", "Invalid HTTP chunk terminator")?;
            }
        } else if let Some(length) = headers.get("content-length") {
            let count = length.parse::<usize>()?;
            require(count <= 64 * 1024 * 1024, "Fixture body limit")?;
            body.resize(count, 0);
            input.read_exact(&mut body)?;
        }
        let mut record = json!({"method":method,"path":target,"headers":headers,"body":String::from_utf8_lossy(&body)});
        if target.starts_with("/host/") {
            record["body_bytes"] = json!(body);
        }
        Ok(Some(Self {
            method,
            target,
            headers,
            body,
            record: Arc::new(Mutex::new(record)),
        }))
    }

    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers.get(&key.to_lowercase()).map(String::as_str)
    }
    pub fn url(&self, origin: &str) -> Result<reqwest::Url> {
        Ok(reqwest::Url::parse(&if self.target.starts_with("http://")
            || self.target.starts_with("https://")
        {
            self.target.clone()
        } else {
            format!("{origin}{}", self.target)
        })?)
    }
}

pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

impl Reply {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
            headers: Vec::new(),
        }
    }
    pub fn json(status: u16, value: &Value) -> Self {
        Self::new(status, crate::json::stringify(value)).header("Content-Type", "application/json")
    }
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }
    pub fn write(&self, stream: &mut TcpStream, head: bool) -> Result {
        let status = reqwest::StatusCode::from_u16(self.status)?;
        write!(
            stream,
            "HTTP/1.0 {} {}\r\n",
            self.status,
            status.canonical_reason().unwrap_or("Fixture")
        )?;
        for (key, value) in &self.headers {
            require(
                !key.contains(['\r', '\n']) && !value.contains(['\r', '\n']),
                "Invalid fixture response header",
            )?;
            write!(stream, "{key}: {value}\r\n")?;
        }
        write!(stream, "Content-Length: {}\r\n\r\n", self.body.len())?;
        if !head {
            stream.write_all(&self.body)?;
        }
        Ok(())
    }
}

type Records = Arc<Mutex<Vec<Arc<Mutex<Value>>>>>;

pub struct Fixture {
    pub origin: String,
    records: Records,
    errors: Arc<Mutex<Vec<String>>>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Fixture {
    pub fn new(
        port: u16,
        handler: impl Fn(&Request, &str) -> Result<Reply> + Send + Sync + 'static,
    ) -> Result<Self> {
        Self::raw(port,move |request,origin,stream| {
            let reply=handler(request,origin)?;
            // Timeout/cancellation fixtures intentionally close before the response.
            let _=reply.write(stream,request.method=="HEAD");
            Ok(())
        })
    }

    pub fn raw(
        port:u16,
        handler: impl Fn(&Request,&str,&mut TcpStream)->Result + Send + Sync + 'static,
    )->Result<Self> {
        actions_only();
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        listener.set_nonblocking(true)?;
        let origin = format!("http://{}", listener.local_addr()?);
        let records: Records = Arc::default();
        let errors = Arc::new(Mutex::new(Vec::new()));
        let stopping = Arc::new(AtomicBool::new(false));
        let (url, recorded, failures, stop) = (
            origin.clone(),
            records.clone(),
            errors.clone(),
            stopping.clone(),
        );
        let handler = Arc::new(handler);
        let thread = std::thread::spawn(move || {
            let mut workers = Vec::new();
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let (handler, url, recorded, failures) = (
                            handler.clone(),
                            url.clone(),
                            recorded.clone(),
                            failures.clone(),
                        );
                        workers.push(std::thread::spawn(move || {
                            let result = (|| -> Result {
                                // Accepted sockets inherit nonblocking mode on Windows.
                                stream.set_nonblocking(false)?;
                                stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                                stream.set_write_timeout(Some(Duration::from_secs(5)))?;
                                if let Some(request) = Request::read(&mut stream)? {
                                    recorded.lock().unwrap().push(request.record.clone());
                                    handler(&request, &url,&mut stream)?;
                                }
                                Ok(())
                            })();
                            if let Err(error) = result {
                                failures.lock().unwrap().push(error.to_string());
                            }
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => {
                        failures.lock().unwrap().push(error.to_string());
                        break;
                    }
                }
            }
            for worker in workers {
                if worker.join().is_err() {
                    failures
                        .lock()
                        .unwrap()
                        .push("Fixture worker panicked".into());
                }
            }
        });
        Ok(Self {
            origin,
            records,
            errors,
            stopping,
            thread: Some(thread),
        })
    }
    pub fn snapshot(&self) -> Vec<Value> {
        self.records
            .lock()
            .unwrap()
            .iter()
            .map(|item| item.lock().unwrap().clone())
            .collect()
    }
    pub fn count(&self) -> usize {
        self.records.lock().unwrap().len()
    }
    pub fn since(&self, offset: usize) -> Vec<Value> {
        self.records.lock().unwrap()[offset..]
            .iter()
            .map(|item| item.lock().unwrap().clone())
            .collect()
    }
    pub fn healthy(&self) -> Result {
        require(
            self.errors.lock().unwrap().is_empty(),
            "Controlled fixture failed; inspect recorded requests",
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn unquote(text: &str) -> Result<String> {
    let mut result = Vec::new();
    let mut bytes = text.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let digits = [
                bytes.next().ok_or("Truncated percent escape")?,
                bytes.next().ok_or("Truncated percent escape")?,
            ];
            result.push(u8::from_str_radix(std::str::from_utf8(&digits)?, 16)?);
        } else {
            result.push(byte);
        }
    }
    Ok(String::from_utf8(result)?)
}

pub fn quote(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn accepted_connection_waits_for_delayed_request_bytes() {
        let fixture=Fixture::new(0,|_,_|Ok(Reply::new(200,"complete"))).unwrap();
        let mut connection=TcpStream::connect(fixture.origin.strip_prefix("http://").unwrap()).unwrap();
        connection.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        std::thread::sleep(Duration::from_millis(80));
        connection.write_all(b"GET /delayed HTTP/1.0\r\nHost: fixture\r\n\r\n").unwrap();
        let mut response=String::new();connection.read_to_string(&mut response).unwrap();assert!(response.starts_with("HTTP/1.0 200 OK\r\n"));assert!(response.ends_with("complete"));assert_eq!(fixture.count(),1);fixture.healthy().unwrap();
    }
}
