//! The camera page and the mirror, served while a session runs. With
//! `--phone`, the camera page is also served over HTTPS for a phone on the
//! network (browsers only open the camera on localhost or HTTPS).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{Context, Result};
use server::phone::Phone;
use server::tls::{self, Identity};
use tokio::task::JoinHandle;

/// HTTPS, for the phone.
pub const PHONE_PORT: u16 = 8443;

/// Serve the screens. `None` if the camera page can't be served.
pub async fn start(
    listen: SocketAddr,
    phone: bool,
    session: server::Live,
    api: Option<server::Api>,
) -> Option<Vec<JoinHandle<()>>> {
    let mut session = session;
    let mut http = listen;
    let mut https = None;
    if phone {
        match phone_identity() {
            Ok((id, ip)) => {
                session = session.with_phone(Phone {
                    origin: format!("https://{ip}:{PHONE_PORT}"),
                    ca_der: id.ca_der.clone(),
                });
                // The phone opens its setup page over plain HTTP, before it
                // trusts the certificate.
                http.set_ip(Ipv4Addr::UNSPECIFIED.into());
                https = Some((id, ip));
            }
            Err(e) => eprintln!("Phone camera unavailable: {e:#}"),
        }
    }
    let app = server::router(api, &crate::default_web_dir(), session);
    let (addr, task) = match server::listen(http, app.clone()).await {
        Ok(x) => x,
        Err(e) => {
            eprintln!("Camera page unavailable ({e:#}); is `rowctl serve` already using that port?");
            return None;
        }
    };
    let mut tasks = vec![task];
    let port = addr.port();
    let Some((id, ip)) = https else {
        println!("Camera page: http://{addr}/camera (open it on this machine: browsers only allow the camera on localhost; for a phone, use --phone)");
        if addr.ip().is_loopback() {
            println!("Mirror: http://{addr}/mirror (for a screen on another machine, use --listen 0.0.0.0:{port})");
        } else {
            println!("Mirror: http://<this machine's address>:{port}/mirror");
        }
        return Some(tasks);
    };
    match tls::listen(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), PHONE_PORT), app, id.config).await {
        Ok((_, task)) => {
            tasks.push(task);
            let setup = format!("http://{ip}:{port}/phone");
            println!("Phone camera: scan this with the phone, or open {setup}");
            print_qr(&setup);
            println!("Mirror: http://localhost:{port}/mirror (another screen: http://{ip}:{port}/mirror)");
            println!("Serving on the network: any device on it can open the review screen.");
        }
        Err(e) => eprintln!("Phone camera unavailable ({e:#}); is port {PHONE_PORT} in use?"),
    }
    Some(tasks)
}

fn phone_identity() -> Result<(Identity, IpAddr)> {
    let ip = tls::lan_ip().context("no network address: is this machine on Wi-Fi or Ethernet?")?;
    let dir = store::paths::data_dir().join("tls");
    let id = tls::identity(&dir, &[ip, Ipv4Addr::LOCALHOST.into()])?;
    Ok((id, ip))
}

/// Light modules on the terminal's dark background, with a light quiet zone.
fn print_qr(text: &str) {
    use qrcode::render::unicode::Dense1x2;
    if let Ok(code) = qrcode::QrCode::new(text) {
        let art = code
            .render::<Dense1x2>()
            .dark_color(Dense1x2::Light)
            .light_color(Dense1x2::Dark)
            .build();
        println!("{art}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_qr_code_renders() {
        let code = qrcode::QrCode::new("http://192.168.1.20:8080/phone").unwrap();
        let art = code.render::<qrcode::render::unicode::Dense1x2>().build();
        assert!(art.lines().count() > 10);
    }
}
