//! Native transport options used by subscription downloads and script HTTP.
use base64::Engine;
use hyper::{
    Uri,
    rt::{Read, ReadBufCursor, Write},
};
use hyper_util::{
    client::legacy::connect::{Connected, Connection, HttpConnector},
    rt::TokioIo,
};
use percent_encoding::percent_decode_str;
use reqwest::Url;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use serde_json::Value;
use std::{
    error::Error,
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_rustls::{TlsConnector, rustls};
use tower_service::Service;

type Failure = Box<dyn Error + Send + Sync>;

pub(super) fn max_header_size() -> usize {
    parse_header_size(&std::env::var("SUB_STORE_MAX_HEADER_SIZE").unwrap_or_default())
}

fn parse_header_size(value: &str) -> usize {
    let value = value.trim_matches(|c: char| c == '\u{feff}' || c != '\u{85}' && c.is_whitespace());
    let value = value.strip_prefix('+').unwrap_or(value);
    let digits = value.bytes().take_while(u8::is_ascii_digit).count();
    value[..digits]
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(32 * 1024)
}

trait Socket: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Socket for T {}

pub(super) struct Stream {
    io: TokioIo<Box<dyn Socket>>,
    proxied: bool,
}

impl Connection for Stream {
    fn connected(&self) -> Connected {
        Connected::new().proxy(self.proxied)
    }
}

impl Read for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        Read::poll_read(Pin::new(&mut self.get_mut().io), cx, buf)
    }
}

impl Write for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Write::poll_write(Pin::new(&mut self.get_mut().io), cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Write::poll_flush(Pin::new(&mut self.get_mut().io), cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Write::poll_shutdown(Pin::new(&mut self.get_mut().io), cx)
    }
}

#[derive(Debug)]
struct SkipCertificateChain(Arc<dyn ServerCertVerifier>);

impl ServerCertVerifier for SkipCertificateChain {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, signature)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

fn new_platform_verifier() -> Result<Arc<dyn ServerCertVerifier>, Failure> {
    Ok(Arc::new(rustls_platform_verifier::Verifier::new(
        Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
    )?))
}

fn platform_verifier() -> Result<Arc<dyn ServerCertVerifier>, Failure> {
    static VERIFIER: Mutex<Option<Arc<dyn ServerCertVerifier>>> = Mutex::new(None);
    let mut cached = VERIFIER
        .lock()
        .map_err(|_| "TLS verifier initialization lock poisoned")?;
    if let Some(verifier) = cached.as_ref() {
        return Ok(verifier.clone());
    }
    // Linux loads/parses the system roots here. Retain successful initialization;
    // each request still owns its TLS config/session state and verification policy.
    let verifier = new_platform_verifier()?;
    *cached = Some(verifier.clone());
    Ok(verifier)
}

fn tls_config(insecure: bool) -> Result<rustls::ClientConfig, Failure> {
    let verifier = platform_verifier()?;
    let verifier = if insecure {
        Arc::new(SkipCertificateChain(verifier)) as Arc<dyn ServerCertVerifier>
    } else {
        verifier
    };
    Ok(rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth())
}

fn environment(lower: &str, upper: &str) -> String {
    std::env::var(lower)
        .or_else(|_| std::env::var(upper))
        .unwrap_or_default()
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        _ => true,
    }
}

fn bypass(uri: &Uri) -> bool {
    let raw = environment("no_proxy", "NO_PROXY");
    if raw == "*" {
        return true;
    }
    let mut host = uri.host().unwrap_or_default().to_ascii_lowercase();
    if host.contains(':') && !host.starts_with('[') {
        host = format!("[{host}]");
    }
    let port = uri
        .port_u16()
        .unwrap_or(if uri.scheme_str() == Some("https") {
            443
        } else {
            80
        });
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            let (name, requested) = match entry.rsplit_once(':') {
                Some((name, port))
                    if !port.is_empty() && port.bytes().all(|c| c.is_ascii_digit()) =>
                {
                    (name, port.parse::<u64>().unwrap_or(u64::MAX))
                }
                _ => (entry, 0),
            };
            let name = name
                .strip_prefix("*.")
                .or_else(|| name.strip_prefix('.'))
                .unwrap_or(name)
                .to_ascii_lowercase();
            (requested == 0 || requested == u64::from(port))
                && (host == name || host.ends_with(&format!(".{name}")))
        })
}

pub(super) fn authorization(proxy: &Url) -> Result<Option<String>, Failure> {
    if proxy.username().is_empty() {
        return Ok(None);
    }
    let user = percent_decode_str(proxy.username()).decode_utf8()?;
    let password = percent_decode_str(proxy.password().unwrap_or_default()).decode_utf8()?;
    Ok(Some(format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
    )))
}

#[derive(Clone)]
pub(super) struct Transport {
    proxy: Option<Url>,
    environment: bool,
    tls: Arc<rustls::ClientConfig>,
}

impl Transport {
    pub(super) fn new(options: Option<&Value>) -> Result<Self, Failure> {
        let proxy = options
            .and_then(|value| value.get("proxy"))
            .filter(|value| truthy(value));
        let proxy = proxy
            .map(|value| {
                value
                    .as_str()
                    .ok_or("Invalid proxy URL")
                    .and_then(|value| Url::parse(value).map_err(|_| "Invalid proxy URL"))
            })
            .transpose()?;
        let insecure = options.is_some_and(|value| {
            value.get("insecure") == Some(&Value::Bool(true))
                || value.get("strictSSL") == Some(&Value::Bool(false))
                || value.get("rejectUnauthorized") == Some(&Value::Bool(false))
        });
        Ok(Self {
            proxy,
            environment: options.is_some(),
            tls: Arc::new(tls_config(insecure)?),
        })
    }

    pub(super) fn proxy(&self, uri: &Uri) -> Result<Option<Url>, Failure> {
        if let Some(proxy) = &self.proxy {
            return Ok(Some(proxy.clone()));
        }
        if !self.environment {
            return Ok(None);
        }
        let all = std::env::var("all_proxy")
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| std::env::var("ALL_PROXY").ok())
            .unwrap_or_default();
        if all.starts_with("socks5://") {
            return Ok(Some(Url::parse(&all)?));
        }
        if bypass(uri) {
            return Ok(None);
        }
        let http = environment("http_proxy", "HTTP_PROXY");
        let https = environment("https_proxy", "HTTPS_PROXY");
        let proxy = if uri.scheme_str() == Some("https") && !https.is_empty() {
            https
        } else {
            http
        };
        if proxy.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Url::parse(&proxy)?))
        }
    }

    pub(super) fn connector(&self) -> hyper_rustls::HttpsConnector<Self> {
        hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config((*self.tls).clone())
            .https_or_http()
            .enable_http1()
            .wrap_connector(self.clone())
    }

    async fn connect(&self, uri: Uri) -> Result<Stream, Failure> {
        let host = uri
            .host()
            .ok_or("Missing HTTP host")?
            .trim_matches(['[', ']']);
        let port = uri
            .port_u16()
            .unwrap_or(if uri.scheme_str() == Some("https") {
                443
            } else {
                80
            });
        let proxy = self.proxy(&uri)?;
        let (connect_host, connect_port) = proxy.as_ref().map_or((host, port), |proxy| {
            (
                proxy
                    .host_str()
                    .unwrap_or_default()
                    .trim_matches(['[', ']']),
                proxy
                    .port_or_known_default()
                    .unwrap_or(if proxy.scheme() == "socks5" { 1080 } else { 80 }),
            )
        });
        let authority = if connect_host.contains(':') {
            format!("[{connect_host}]:{connect_port}")
        } else {
            format!("{connect_host}:{connect_port}")
        };
        let destination = Uri::builder()
            .scheme("http")
            .authority(authority)
            .path_and_query("/")
            .build()?;
        let mut connector = HttpConnector::new();
        connector.set_happy_eyeballs_timeout(Some(Duration::from_millis(250)));
        let mut socket: Box<dyn Socket> = Box::new(connector.call(destination).await?.into_inner());
        let mut proxied = false;
        if let Some(proxy) = proxy.as_ref() {
            if proxy.scheme() == "socks5" {
                socks(&mut socket, proxy, host, port).await?;
            } else if ["http", "https"].contains(&proxy.scheme()) {
                if proxy.scheme() == "https" {
                    let connector = TlsConnector::from(Arc::new(tls_config(false)?));
                    socket = Box::new(
                        connector
                            .connect(ServerName::try_from(connect_host.to_owned())?, socket)
                            .await?,
                    );
                }
                proxied = uri.scheme_str() == Some("http");
                if !proxied {
                    tunnel(&mut socket, proxy, host, port).await?;
                }
            } else {
                return Err(format!("Unsupported proxy protocol: {}", proxy.scheme()).into());
            }
        }
        Ok(Stream {
            io: TokioIo::new(socket),
            proxied,
        })
    }
}

impl Service<Uri> for Transport {
    type Response = Stream;
    type Error = Failure;
    type Future = Pin<Box<dyn Future<Output = Result<Stream, Failure>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let transport = self.clone();
        Box::pin(async move { transport.connect(uri).await })
    }
}

async fn tunnel(
    socket: &mut Box<dyn Socket>,
    proxy: &Url,
    host: &str,
    port: u16,
) -> Result<(), Failure> {
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(auth) = authorization(proxy)? {
        request.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    request.push_str("\r\n");
    socket.write_all(request.as_bytes()).await?;
    let maximum = max_header_size();
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        if headers.len() >= maximum {
            return Err("Headers Overflow Error".into());
        }
        headers.push(socket.read_u8().await?);
    }
    let status = std::str::from_utf8(&headers)?
        .split_ascii_whitespace()
        .nth(1)
        .ok_or("Invalid proxy response")?;
    if status != "200" {
        return Err(format!("Proxy response ({status}) !== 200 when HTTP Tunneling").into());
    }
    Ok(())
}

async fn socks(
    socket: &mut Box<dyn Socket>,
    proxy: &Url,
    host: &str,
    port: u16,
) -> Result<(), Failure> {
    let user = percent_decode_str(proxy.username()).decode_utf8()?;
    let password = percent_decode_str(proxy.password().unwrap_or_default()).decode_utf8()?;
    let credentials = !user.is_empty() || !password.is_empty();
    socket
        .write_all(if credentials {
            &[5, 2, 0, 2]
        } else {
            &[5, 1, 0]
        })
        .await?;
    let mut answer = [0; 2];
    socket.read_exact(&mut answer).await?;
    if answer[0] != 5 {
        return Err("Invalid SOCKS proxy response".into());
    }
    match answer[1] {
        0 => {}
        2 if credentials => {
            let mut auth = vec![1, u8::try_from(user.len())?];
            auth.extend_from_slice(user.as_bytes());
            auth.push(u8::try_from(password.len())?);
            auth.extend_from_slice(password.as_bytes());
            socket.write_all(&auth).await?;
            socket.read_exact(&mut answer).await?;
            if answer != [1, 0] {
                return Err("SOCKS proxy authentication failed".into());
            }
        }
        _ => return Err("SOCKS proxy rejected authentication".into()),
    }
    let mut request = vec![5, 1, 0];
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            request.push(4);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            request.extend([3, u8::try_from(host.len())?]);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    socket.write_all(&request).await?;
    let mut answer = [0; 4];
    socket.read_exact(&mut answer).await?;
    if answer[0] != 5 || answer[1] != 0 {
        return Err(format!("SOCKS proxy connection failed: {}", answer[1]).into());
    }
    let length = match answer[3] {
        1 => 4,
        4 => 16,
        3 => usize::from(socket.read_u8().await?),
        _ => return Err("Invalid SOCKS proxy address".into()),
    };
    socket.read_exact(&mut vec![0; length + 2]).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_limit_uses_positive_decimal_prefix_and_default() {
        for value in ["65536", " +65536junk", "\u{feff}65536\u{feff}"] {
            assert_eq!(parse_header_size(value), 65536);
        }
        assert_eq!(parse_header_size("1e3"), 1);
        for value in ["", "0", "-1", "invalid", "Infinity", "\u{85}65536"] {
            assert_eq!(parse_header_size(value), 32768);
        }
    }

    #[test]
    fn concurrent_clients_reuse_roots_without_weakening_certificate_verification() {
        let threads: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(platform_verifier))
            .collect();
        let verifiers: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap().unwrap())
            .collect();
        let verifier = &verifiers[0];
        assert!(verifiers.iter().all(|other| Arc::ptr_eq(verifier, other)));
        let invalid = CertificateDer::from(vec![0]);
        let name = ServerName::try_from("fixture.invalid").unwrap();
        let now = UnixTime::since_unix_epoch(Duration::from_secs(1_700_000_000));
        assert!(
            verifier
                .verify_server_cert(&invalid, &[], &name, &[], now)
                .is_err()
        );
        let insecure = SkipCertificateChain(verifier.clone());
        assert!(
            insecure
                .verify_server_cert(&invalid, &[], &name, &[], now)
                .is_ok()
        );
        assert!(
            verifier
                .verify_server_cert(&invalid, &[], &name, &[], now)
                .is_err()
        );
    }

    #[test]
    #[ignore = "Release CPU profile; execute only in GitHub Actions"]
    fn profile_verifier_initialization() {
        use std::{hint::black_box, time::Instant};
        let first = Instant::now();
        black_box(platform_verifier().unwrap());
        println!(
            "platform verifier profile: first_ms={:.3}",
            first.elapsed().as_secs_f64() * 1000.0
        );
        for block in 1..=3 {
            let fresh = Instant::now();
            for _ in 0..100 {
                black_box(new_platform_verifier().unwrap());
            }
            let fresh = fresh.elapsed().as_secs_f64() * 1000.0;
            let shared = Instant::now();
            for _ in 0..100 {
                black_box(platform_verifier().unwrap());
            }
            let shared = shared.elapsed().as_secs_f64() * 1000.0;
            println!(
                "platform verifier profile: block={block}, calls=100, fresh_ms={fresh:.3}, shared_ms={shared:.3}"
            );
        }
    }
}
