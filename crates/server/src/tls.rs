//! HTTPS for a camera page on another device (a phone): browsers only open
//! the camera on localhost or over HTTPS.
//!
//! A local certificate authority is made once and kept in the data directory;
//! the phone is told to trust it once. Every start issues a fresh server
//! certificate from it for this machine's current addresses, so a new
//! address from the router doesn't need the phone set up again.

use std::fs;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use anyhow::{Context, Result};
use axum::Router;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, SanType,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::{Duration, OffsetDateTime};
use tracing::{debug, warn};

const CA_CERT: &str = "ca.pem";
const CA_KEY: &str = "ca-key.pem";
pub const CA_NAME: &str = "Row Assist local CA";
/// iOS rejects server certificates valid for more than 825 days.
pub const SERVER_DAYS: i64 = 397;
const CA_DAYS: i64 = 3650;

/// What the HTTPS listener serves with, and what the phone installs.
pub struct Identity {
    /// The CA certificate (DER), for the phone to trust.
    pub ca_der: Vec<u8>,
    /// This start's server certificate (DER), signed by the CA.
    pub server_der: Vec<u8>,
    pub config: Arc<rustls::ServerConfig>,
}

/// Load the CA from `dir` (making it on first use) and issue a server
/// certificate for `ips` and `localhost`.
pub fn identity(dir: &Path, ips: &[IpAddr]) -> Result<Identity> {
    let (ca_pem, ca_key) = load_or_create_ca(dir)?;
    let ca_der = CertificateDer::from_pem_slice(ca_pem.as_bytes())
        .context("read the CA certificate")?
        .to_vec();
    let issuer = Issuer::from_ca_cert_pem(&ca_pem, ca_key).context("load the CA")?;

    let key = KeyPair::generate()?;
    let mut params = CertificateParams::default();
    params.distinguished_name = name("Row Assist");
    params.subject_alt_names = ips.iter().map(|ip| SanType::IpAddress(*ip)).collect();
    params.subject_alt_names.push(SanType::DnsName("localhost".try_into()?));
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.use_authority_key_identifier_extension = true;
    let now = OffsetDateTime::now_utc();
    // A day of slack for a phone whose clock is behind.
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(SERVER_DAYS);
    let cert = params.signed_by(&key, &issuer)?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )?;
    // WebSockets upgrade over HTTP/1.1.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Identity {
        ca_der,
        server_der: cert.der().to_vec(),
        config: Arc::new(config),
    })
}

fn name(common: &str) -> DistinguishedName {
    let mut dn = DistinguishedName::new();
    dn.push(DnType::OrganizationName, "Row Assist");
    dn.push(DnType::CommonName, common);
    dn
}

fn load_or_create_ca(dir: &Path) -> Result<(String, KeyPair)> {
    let (cert_path, key_path) = (dir.join(CA_CERT), dir.join(CA_KEY));
    if cert_path.is_file() && key_path.is_file() {
        let key = KeyPair::from_pem(&fs::read_to_string(&key_path)?).context("read the CA key")?;
        return Ok((fs::read_to_string(&cert_path)?, key));
    }
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let key = KeyPair::generate()?;
    let now = OffsetDateTime::now_utc();
    let mut params = CertificateParams::default();
    // The date tells a remade CA from an old one in the phone's settings.
    params.distinguished_name = name(&format!("{CA_NAME} {}", now.date()));
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(CA_DAYS);
    let cert = params.self_signed(&key)?;
    write_private(&key_path, key.serialize_pem().as_bytes())?;
    fs::write(&cert_path, cert.pem())?;
    Ok((cert.pem(), key))
}

/// The CA key can vouch for any site to a phone that trusts it: owner-only.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(path).with_context(|| format!("write {}", path.display()))?;
    f.write_all(bytes)?;
    Ok(())
}

/// This machine's address on the local network: the one it would reach the
/// internet from. Nothing is sent.
pub fn lan_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    s.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// Serve `app` over HTTPS in the background, WebSockets included. Returns the
/// bound address.
pub async fn listen(
    addr: SocketAddr,
    app: Router,
    config: Arc<rustls::ServerConfig>,
) -> Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("listen on {addr}"))?;
    let bound = listener.local_addr()?;
    let acceptor = tokio_rustls::TlsAcceptor::from(config);
    let task = tokio::spawn(async move {
        loop {
            let (tcp, peer) = match listener.accept().await {
                Ok(c) => c,
                Err(e) => {
                    warn!("https: accept: {e}");
                    tokio::time::sleep(StdDuration::from_millis(100)).await;
                    continue;
                }
            };
            let (acceptor, app) = (acceptor.clone(), app.clone());
            tokio::spawn(async move {
                // A phone that doesn't trust the CA yet ends here.
                let tls = match acceptor.accept(tcp).await {
                    Ok(s) => s,
                    Err(e) => {
                        debug!("https: handshake with {peer}: {e}");
                        return;
                    }
                };
                let service = hyper_util::service::TowerToHyperService::new(app);
                let conn = hyper::server::conn::http1::Builder::new()
                    .serve_connection(hyper_util::rt::TokioIo::new(tls), service)
                    .with_upgrades();
                if let Err(e) = conn.await {
                    debug!("https: {peer}: {e}");
                }
            });
        }
    });
    Ok((bound, task))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use futures::{SinkExt, StreamExt};
    use model::Clock;
    use rustls::pki_types::ServerName;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio_tungstenite::tungstenite::Message;

    use super::*;
    use crate::Live;
    use crate::phone::Phone;

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("row-assist-tls-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn client(roots: &[&[u8]]) -> tokio_rustls::TlsConnector {
        let mut store = rustls::RootCertStore::empty();
        for der in roots {
            store.add(CertificateDer::from(der.to_vec())).unwrap();
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(store)
            .with_no_client_auth();
        tokio_rustls::TlsConnector::from(Arc::new(cfg))
    }

    #[test]
    fn the_ca_is_made_once_and_server_certificates_suit_ios() {
        let dir = tmp("ca");
        let lan: IpAddr = "192.168.1.20".parse().unwrap();
        let a = identity(&dir, &[lan]).unwrap();
        let b = identity(&dir, &["10.0.0.7".parse().unwrap()]).unwrap();
        assert_eq!(a.ca_der, b.ca_der, "the phone trusts the CA once, whatever the address");
        assert_ne!(a.server_der, b.server_der);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join(CA_KEY)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let (_, ca) = x509_parser::parse_x509_certificate(&a.ca_der).unwrap();
        assert!(ca.is_ca());
        assert!(ca.subject().to_string().contains(CA_NAME));
        let (_, srv) = x509_parser::parse_x509_certificate(&a.server_der).unwrap();
        srv.verify_signature(Some(ca.public_key())).unwrap();
        assert!(!srv.is_ca());
        // iOS: at most 825 days, serverAuth, and the address in the SAN.
        let v = srv.validity();
        assert!((v.not_after.timestamp() - v.not_before.timestamp()) / 86_400 <= 825);
        assert!(srv.extended_key_usage().unwrap().unwrap().value.server_auth);
        let sans = srv.subject_alternative_name().unwrap().unwrap().value.general_names.clone();
        let has_ip = sans.iter().any(|n| matches!(n, x509_parser::extensions::GeneralName::IPAddress(b) if *b == [192, 168, 1, 20]));
        assert!(has_ip, "{sans:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn https_and_the_camera_socket_work_once_the_ca_is_trusted() {
        let dir = tmp("serve");
        let id = identity(&dir, &[IpAddr::V4(Ipv4Addr::LOCALHOST)]).unwrap();
        let phone = Phone {
            origin: "https://127.0.0.1:8443".into(),
            ca_der: id.ca_der.clone(),
        };
        let app = crate::router(None, &dir, Live::setup_only(Clock::start()).with_phone(phone));
        let (addr, server) = listen("127.0.0.1:0".parse().unwrap(), app, id.config.clone()).await.unwrap();
        let name = ServerName::IpAddress(Ipv4Addr::LOCALHOST.into());
        let trusting = client(&[&id.ca_der]);

        // The certificate download.
        let mut tls = trusting.connect(name.clone(), TcpStream::connect(addr).await.unwrap()).await.unwrap();
        tls.write_all(b"GET /phone/ca.cer HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut res = Vec::new();
        let _ = tls.read_to_end(&mut res).await; // the server may close without close_notify
        let head = String::from_utf8_lossy(&res[..res.len().min(200)]).to_lowercase();
        assert!(head.starts_with("http/1.1 200"), "{head}");
        assert!(head.contains("content-type: application/x-x509-ca-cert"));
        assert!(res.ends_with(&id.ca_der));

        // The camera socket upgrades over TLS and answers clock pings.
        let tls = trusting.connect(name.clone(), TcpStream::connect(addr).await.unwrap()).await.unwrap();
        let (mut ws, _) = tokio_tungstenite::client_async("wss://127.0.0.1/ws/camera", tls).await.unwrap();
        ws.send(Message::Text(r#"{"ping": 12.5}"#.into())).await.unwrap();
        let before = Clock::start().now_us();
        let clock = loop {
            let Message::Text(t) = ws.next().await.unwrap().unwrap() else { continue };
            let m: serde_json::Value = serde_json::from_str(&t).unwrap();
            if m["type"] == "clock" {
                break m;
            }
        };
        assert_eq!(clock["ping"], 12.5);
        assert!((clock["server_us"].as_i64().unwrap() - before).abs() < 5_000_000);

        // Before the phone trusts the CA, the handshake fails.
        let untrusting = client(&[]);
        assert!(untrusting.connect(name, TcpStream::connect(addr).await.unwrap()).await.is_err());
        server.abort();
        fs::remove_dir_all(&dir).unwrap();
    }
}
