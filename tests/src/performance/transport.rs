use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    process::{Command, Stdio},
    time::Duration,
};

pub struct Connection {
    socket: BufReader<TcpStream>,
    host: String,
    port: u16,
}
impl Connection {
    pub fn new(origin: &str) -> Result<Self> {
        let url = reqwest::Url::parse(origin)?;
        check(
            url.scheme() == "http" && url.username().is_empty() && url.password().is_none(),
            "http",
        )?;
        let host = url.host_str().ok_or("HTTP host missing")?.to_owned();
        let port = url.port_or_known_default().ok_or("HTTP port missing")?;
        let socket = TcpStream::connect((host.as_str(), port))?;
        socket.set_read_timeout(Some(Duration::from_secs(300)))?;
        socket.set_write_timeout(Some(Duration::from_secs(300)))?;
        socket.set_nodelay(true)?;
        Ok(Self {
            socket: BufReader::new(socket),
            host,
            port,
        })
    }
    pub fn request(
        &mut self,
        method: &str,
        path: &str,
        data: Option<&Value>,
        keep: bool,
    ) -> Result<Vec<u8>> {
        check(
            !method.contains([' ', '\r', '\n'])
                && path.starts_with('/')
                && !path.contains(['\r', '\n']),
            "http",
        )?;
        let body = data.map(canonical).unwrap_or_default();
        let mut wire = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}:{}\r\nConnection: {}\r\nContent-Length: {}\r\n",
            self.host,
            self.port,
            if keep { "keep-alive" } else { "close" },
            body.len()
        )
        .into_bytes();
        if data.is_some() {
            wire.extend_from_slice(b"Content-Type: application/json\r\n");
        }
        wire.extend_from_slice(b"\r\n");
        wire.extend_from_slice(&body);
        self.socket.get_mut().write_all(&wire)?;
        let mut wire = Vec::new();
        let mut length = None;
        let mut chunked = false;
        let mut closed = false;
        let mut status = 0_u16;
        loop {
            let before = wire.len();
            check(
                self.socket.read_until(b'\n', &mut wire)? > 0 && wire.len() <= 1048576,
                "http",
            )?;
            let line = &wire[before..];
            if before == 0 {
                let text = std::str::from_utf8(line)?;
                status = text
                    .split_whitespace()
                    .nth(1)
                    .ok_or("HTTP status missing")?
                    .parse()?;
            } else if line == b"\r\n" {
                break;
            } else if let Some((name, value)) = std::str::from_utf8(line)?.split_once(':') {
                match name.to_ascii_lowercase().as_str() {
                    "content-length" => {
                        check(length.is_none(), "http")?;
                        length = Some(value.trim().parse::<usize>()?);
                    }
                    "transfer-encoding" => chunked = value.trim().eq_ignore_ascii_case("chunked"),
                    "connection" => closed = value.trim().eq_ignore_ascii_case("close"),
                    _ => {}
                }
            }
        }
        if method != "HEAD" && !matches!(status, 204 | 304) {
            if chunked {
                loop {
                    let mut line = Vec::new();
                    check(
                        self.socket.read_until(b'\n', &mut line)? > 0 && line.len() < 1024,
                        "http",
                    )?;
                    let size = usize::from_str_radix(
                        std::str::from_utf8(&line)?
                            .trim()
                            .split(';')
                            .next()
                            .ok_or("Chunk length missing")?,
                        16,
                    )?;
                    wire.extend_from_slice(&line);
                    check(
                        wire.len()
                            .checked_add(size)
                            .is_some_and(|total| total <= 128 * 1024 * 1024),
                        "http",
                    )?;
                    if size == 0 {
                        loop {
                            let mut trailer = Vec::new();
                            check(
                                self.socket.read_until(b'\n', &mut trailer)? > 0
                                    && trailer.len() <= 1048576,
                                "http",
                            )?;
                            wire.extend_from_slice(&trailer);
                            if trailer == b"\r\n" {
                                break;
                            }
                        }
                        break;
                    }
                    let before = wire.len();
                    wire.resize(before + size + 2, 0);
                    self.socket.read_exact(&mut wire[before..])?;
                    check(&wire[wire.len() - 2..] == b"\r\n", "http")?;
                }
            } else if let Some(length) = length {
                check(length <= 128 * 1024 * 1024, "http")?;
                let before = wire.len();
                wire.resize(before + length, 0);
                self.socket.read_exact(&mut wire[before..])?;
            } else {
                check(!keep, "http")?;
                self.socket
                    .by_ref()
                    .take(128 * 1024 * 1024)
                    .read_to_end(&mut wire)?;
            }
        }
        check(!keep || !closed, "http")?;
        let reply = crate::raw_http::decode(&wire, method == "HEAD")?;
        if !(200..300).contains(&reply.status) {
            let text = String::from_utf8_lossy(&reply.body);
            return Err(Box::new(Failure {
                code: "http",
                observations: Some(
                    json!({"http_status":reply.status,"response_sha256":digest(&reply.body),"empty_nodes":text.contains("不含有效节点"),"detector_start_failed":text.contains("HTTP-META /start failed"),"cache_unavailable":text.contains("scriptResourceCache unavailable"),"snapshot_error":text.contains("HealthSnapshot")||text.contains("Snapshot")}),
                ),
            }));
        }
        Ok(reply.body)
    }
}
pub fn http(url: &str, method: &str, data: Option<&Value>) -> Result<Vec<u8>> {
    let parsed = reqwest::Url::parse(url)?;
    let mut path = parsed.path().to_owned();
    if let Some(query) = parsed.query() {
        path.push('?');
        path.push_str(query);
    }
    let result = Connection::new(url)?.request(method, &path, data, false);
    match result {
        Err(error)
            if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                )
            }) =>
        {
            Err(Box::new(Failure {
                code: "timeout",
                observations: None,
            }))
        }
        result => result,
    }
}
pub fn api(origin: &str, path: &str, method: &str, data: Option<&Value>) -> Result<Value> {
    let payload: Value = serde_json::from_slice(&http(
        &format!("{origin}/backend/api/{path}"),
        method,
        data,
    )?)?;
    check(payload["status"] == "success", "http")?;
    Ok(payload["data"].clone())
}
pub fn pipe(
    command: &mut Command,
    input: Vec<u8>,
    output: &Path,
    label: &str,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let mut child = crate::process::Running::spawn(command.stdin(Stdio::piped()), output, label)?;
    write(
        output.join(format!("{label}.ownership.json")),
        &json!({"pid":child.child.id(),"direct_reason":"Owned binary pipes require the actual PID, watchdog and byte-preserving stdout"}),
    )?;
    let mut stdin = child.child.stdin.take().ok_or("Child stdin missing")?;
    let writer = std::thread::spawn(move || -> std::io::Result<()> { stdin.write_all(&input) });
    let (status, timed_out) = child.wait(timeout)?;
    let written = writer.join().map_err(|_| "Child writer panicked")?;
    check(status.success() && !timed_out, "capture")?;
    written?;
    Ok(std::fs::read(&child.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistent_socket_identity_bytes_and_private_http_failures_are_checked() -> Result {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let worker = std::thread::spawn(move || -> Result {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut reader = BufReader::new(stream.try_clone()?);
            for index in 0..3 {
                let mut line = String::new();
                loop {
                    line.clear();
                    reader.read_line(&mut line)?;
                    if line == "\r\n" {
                        break;
                    }
                }
                let body = if index == 2 {
                    b"private-credential".as_slice()
                } else {
                    b"exact bytes\0\xff".as_slice()
                };
                write!(
                    stream,
                    "HTTP/1.1 {} OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                    if index == 2 { 404 } else { 200 },
                    body.len()
                )?;
                stream.write_all(body)?;
            }
            Ok(())
        });
        let mut connection = Connection::new(&format!("http://{address}"))?;
        for _ in 0..2 {
            assert_eq!(
                connection.request("GET", "/", None, true)?,
                b"exact bytes\0\xff"
            );
        }
        let error = connection
            .request("GET", "/missing", None, true)
            .unwrap_err();
        let proof = failure(error.as_ref());
        public(&proof, &BTreeSet::new())?;
        assert_eq!(proof["failure_observations"]["http_status"], 404);
        assert!(!serde_json::to_string(&proof)?.contains("private-credential"));
        worker
            .join()
            .map_err(|_| "HTTP self-check server panicked")??;
        Ok(())
    }
}
