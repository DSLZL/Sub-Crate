//! Raw DNS transport for the contained script host; packets stay in dns-packet.
use hyper_util::client::legacy::connect::HttpConnector;
use rquickjs::{Ctx, Exception};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use serde_json::json;
use std::{error::Error, io, sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UdpSocket;
use tokio_rustls::{TlsConnector, rustls};
use tower_service::Service;

pub fn parse_url(ctx: Ctx<'_>, raw: String) -> rquickjs::Result<String> {
    let parsed = reqwest::Url::parse(&raw)
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    Ok(json!({"hostname": parsed.host_str().unwrap_or_default(),
        "port": parsed.port().map(|port| port.to_string()).unwrap_or_default(),
        "username": parsed.username(), "password": parsed.password().unwrap_or_default(),
        "pathname": parsed.path(), "search": parsed.query().unwrap_or_default(),
        "hash": parsed.fragment().unwrap_or_default()})
    .to_string())
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

async fn framed_query<S: AsyncRead + AsyncWrite + Unpin>(
    mut socket: S,
    packet: &[u8],
    timeout: Duration,
    name: &str,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let length = u16::try_from(packet.len())?.to_be_bytes();
    let request = [length.as_slice(), packet].concat();
    let timeout_error = || format!("DNS {name} query timeout");
    tokio::time::timeout(timeout, socket.write_all(&request))
        .await
        .map_err(|_| timeout_error())??;
    let mut response = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        // The reference socket timeout resets on activity, not on the whole query.
        let size = tokio::time::timeout(timeout, socket.read(&mut chunk))
            .await
            .map_err(|_| timeout_error())?
            .or_else(|error| {
                if error.kind() == io::ErrorKind::UnexpectedEof {
                    Ok(0)
                } else {
                    Err(error)
                }
            })?;
        if size == 0 {
            return Err(format!("DNS {name} connection ended before response").into());
        }
        response.extend_from_slice(&chunk[..size]);
        if response.len() >= 2 {
            let length = usize::from(u16::from_be_bytes([response[0], response[1]]));
            if response.len() >= length + 2 {
                return Ok(response[2..length + 2].to_vec());
            }
        }
    }
}

pub async fn query<'js>(
    ctx: Ctx<'js>,
    server: String,
    payload: String,
    timeout: f64,
    insecure: bool,
) -> rquickjs::Result<String> {
    async fn send(
        server: &str,
        payload: &str,
        timeout: f64,
        insecure: bool,
    ) -> Result<String, Box<dyn Error>> {
        let server: serde_json::Value = serde_json::from_str(server)?;
        let protocol = server["protocol"].as_str().ok_or("Missing DNS protocol")?;
        let host = server["host"]
            .as_str()
            .filter(|host| !host.is_empty())
            .ok_or("Missing DNS host")?;
        let port = server["port"]
            .as_u64()
            .filter(|port| (1..=65535).contains(port))
            .ok_or("Invalid DNS port")? as u16;
        let packet: Vec<u8> = serde_json::from_str(payload)?;
        if packet.len() > usize::from(u16::MAX) {
            return Err("DNS query exceeds 65535 bytes".into());
        }
        // Node timers clamp invalid/out-of-range values to 1 ms after source defaults.
        let millis = if timeout.is_finite() && (1.0..=2_147_483_647.0).contains(&timeout) {
            timeout as u64
        } else {
            1
        };
        let timeout = Duration::from_millis(millis);
        let result = match protocol {
            "udp" => {
                let exchange = async {
                    let ipv6 = host.parse::<std::net::Ipv6Addr>().is_ok();
                    let socket = UdpSocket::bind(if ipv6 { "[::]:0" } else { "0.0.0.0:0" }).await?;
                    // Node's udp4 socket resolves hostnames to IPv4 even on dual-stack hosts.
                    let address = tokio::net::lookup_host((host, port))
                        .await?
                        .find(|address| address.is_ipv6() == ipv6)
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::AddrNotAvailable,
                                "No DNS server address for socket family",
                            )
                        })?;
                    socket.send_to(&packet, address).await?;
                    let mut response = vec![0; 65535];
                    let (size, _) = socket.recv_from(&mut response).await?;
                    response.truncate(size);
                    Ok::<_, io::Error>(response)
                };
                tokio::time::timeout(timeout, exchange)
                    .await
                    .map_err(|_| "DNS UDP query timeout")??
            }
            "tcp" | "tls" => {
                let name = if protocol == "tls" { "TLS" } else { "TCP" };
                let connect_started = std::time::Instant::now();
                let mut connector = HttpConnector::new();
                // Match the pinned Node default without waiting for a failed address family.
                connector.set_happy_eyeballs_timeout(Some(Duration::from_millis(250)));
                let authority = if host.contains(':') {
                    format!("[{host}]:{port}")
                } else {
                    format!("{host}:{port}")
                };
                let destination = hyper::Uri::builder()
                    .scheme("http")
                    .authority(authority)
                    .path_and_query("/")
                    .build()?;
                let socket = tokio::time::timeout(timeout, connector.call(destination))
                    .await
                    .map_err(|_| format!("DNS {name} query timeout"))?
                    .map_err(|error| {
                        error
                            .source()
                            .map_or_else(|| error.to_string(), ToString::to_string)
                    })?
                    .into_inner();
                eprintln!(
                    "COMPAT_DNS_CONNECT {}",
                    json!({"host": host,
                    "elapsed_us": connect_started.elapsed().as_micros(), "peer": socket.peer_addr().ok()})
                );
                if protocol == "tls" {
                    let verifier: Arc<dyn ServerCertVerifier> =
                        Arc::new(rustls_platform_verifier::Verifier::new(Arc::new(
                            rustls::crypto::aws_lc_rs::default_provider(),
                        ))?);
                    let verifier = if insecure {
                        Arc::new(SkipCertificateChain(verifier)) as Arc<dyn ServerCertVerifier>
                    } else {
                        verifier
                    };
                    let config = rustls::ClientConfig::builder()
                        .dangerous()
                        .with_custom_certificate_verifier(verifier)
                        .with_no_client_auth();
                    let connector = TlsConnector::from(Arc::new(config));
                    let socket = tokio::time::timeout(
                        timeout,
                        connector.connect(ServerName::try_from(host.to_owned())?, socket),
                    )
                    .await
                    .map_err(|_| "DNS TLS query timeout")??;
                    framed_query(socket, &packet, timeout, name).await?
                } else {
                    framed_query(socket, &packet, timeout, name).await?
                }
            }
            _ => return Err("Unsupported native DNS protocol".into()),
        };
        Ok(serde_json::to_string(&result)?)
    }
    send(&server, &payload, timeout, insecure)
        .await
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))
}
