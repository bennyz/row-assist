//! The pages served while rowing: the camera page and the mirror. With a
//! phone, the camera page is also served over HTTPS (browsers only open the
//! camera on localhost or HTTPS), with a certificate for this machine's
//! network addresses that follows them when they change.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use server::phone::Phone;
use server::tls;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// HTTPS, for the phone.
pub const PHONE_PORT: u16 = 8443;
/// How often to look for a new network address.
const ADDRESS_EVERY: Duration = Duration::from_secs(10);

/// Serve the screens. `None` if the pages can't be served.
pub async fn start(
    listen: SocketAddr,
    phone: bool,
    session: server::Live,
    api: Option<server::Api>,
    web: &Path,
) -> Option<Vec<JoinHandle<()>>> {
    let mut http = listen;
    if phone {
        // The phone opens its setup page over plain HTTP, before it trusts
        // the certificate.
        http.set_ip(Ipv4Addr::UNSPECIFIED.into());
    }
    let app = server::router(api, web, session.clone());
    let (addr, task) = match server::listen(http, app.clone()).await {
        Ok(x) => x,
        Err(e) => {
            eprintln!("Pages unavailable ({e:#}); is Row Assist already running (rowctl serve)?");
            return None;
        }
    };
    let mut tasks = vec![task];
    let port = addr.port();
    if !phone {
        println!("Camera page: http://localhost:{port}/camera (on this machine: browsers only allow the camera on localhost; for a phone, use --phone)");
        println!("Mirror: http://localhost:{port}/mirror");
        return Some(tasks);
    }
    let (config_tx, config) = watch::channel(None);
    match tls::listen(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), PHONE_PORT), app, config).await {
        Ok((_, task)) => {
            tasks.push(task);
            tasks.push(tokio::spawn(follow_address(session, config_tx, port)));
            println!("Mirror: http://localhost:{port}/mirror");
            println!("Other devices on the network get only the phone's pages and the Watch upload.");
        }
        Err(e) => eprintln!("Phone camera unavailable ({e:#}); is port {PHONE_PORT} in use?"),
    }
    Some(tasks)
}

/// Keep the phone's certificate and setup address on this machine's current
/// network address: none yet at login before Wi-Fi is up, a new one from the
/// router, another network. The certificate names every address the machine
/// has, so a phone that holds any of them keeps working.
async fn follow_address(session: server::Live, config: watch::Sender<Option<Arc<tls::ServerConfig>>>, port: u16) {
    let dir = store::paths::data_dir().join("tls");
    // The address the phone is given, and the addresses the certificate names.
    let mut current: Option<(IpAddr, Vec<IpAddr>)> = None;
    let mut waiting_said = false;
    loop {
        let ips = tls::local_ips();
        let shown = current.as_ref().map(|(ip, _)| *ip);
        match pick_address(shown, &ips, tls::lan_ip()) {
            Some(ip) if current.as_ref() != Some(&(ip, ips.clone())) => {
                let mut names = ips.clone();
                names.push(Ipv4Addr::LOCALHOST.into());
                match tls::identity(&dir, &names) {
                    Ok(id) => {
                        let setup_url = format!("http://{ip}:{port}/phone");
                        session.set_phone(Some(Phone {
                            origin: format!("https://{ip}:{PHONE_PORT}"),
                            setup_url: setup_url.clone(),
                            ca_der: id.ca_der,
                        }));
                        let _ = config.send(Some(id.config));
                        if shown != Some(ip) {
                            println!("Phone camera: scan this with the phone, or open {setup_url}");
                            print_qr(&setup_url);
                        }
                        current = Some((ip, ips));
                    }
                    Err(e) => {
                        eprintln!("Phone camera unavailable: {e:#}");
                        return;
                    }
                }
            }
            None if current.is_none() && !waiting_said => {
                println!("Phone camera: waiting for a network address (is Wi-Fi on?)");
                waiting_said = true;
            }
            _ => {}
        }
        tokio::time::sleep(ADDRESS_EVERY).await;
    }
}

/// The address to give the phone: the one it already has while this machine
/// still holds it (Ethernet and Wi-Fi on one network take turns as the
/// preferred route), else the one the route prefers.
fn pick_address(shown: Option<IpAddr>, ips: &[IpAddr], routed: Option<IpAddr>) -> Option<IpAddr> {
    shown.filter(|ip| ips.contains(ip)).or(routed)
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
    use super::*;

    #[test]
    fn the_phone_keeps_its_address_while_the_machine_has_it() {
        let (wired, wifi): (IpAddr, IpAddr) = ("10.0.0.22".parse().unwrap(), "10.0.0.13".parse().unwrap());
        let both = [wifi, wired];
        assert_eq!(pick_address(None, &both, Some(wired)), Some(wired));
        // The route flips to Wi-Fi: the phone keeps the wired address.
        assert_eq!(pick_address(Some(wired), &both, Some(wifi)), Some(wired));
        // The cable comes out: Wi-Fi it is.
        assert_eq!(pick_address(Some(wired), &[wifi], Some(wifi)), Some(wifi));
        assert_eq!(pick_address(Some(wifi), &[], None), None);
    }

    #[test]
    fn the_qr_code_renders() {
        let code = qrcode::QrCode::new("http://192.168.1.20:8080/phone").unwrap();
        let art = code.render::<qrcode::render::unicode::Dense1x2>().build();
        assert!(art.lines().count() > 10);
    }
}
