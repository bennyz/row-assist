//! A phone as the side camera: `/api/phone` tells the setup page where the
//! camera page is served over HTTPS, `/phone/ca.cer` is the certificate the
//! phone installs to trust it, and `/api/phone/qr.svg` is the code the laptop
//! shows for the phone to scan.

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::camera::Live;

/// Served with `rowctl serve` and `rowctl record --phone`.
#[derive(Debug, Clone)]
pub struct Phone {
    /// The HTTPS origin, e.g. `https://192.168.1.20:8443`.
    pub origin: String,
    /// The plain-HTTP setup page the phone opens first, e.g.
    /// `http://192.168.1.20:8080/phone`.
    pub setup_url: String,
    /// The CA certificate (DER).
    pub ca_der: Vec<u8>,
}

#[derive(Serialize)]
struct Info<'a> {
    origin: &'a str,
    setup_url: &'a str,
    camera_url: String,
    ca_url: &'static str,
}

pub const CA_PATH: &str = "/phone/ca.cer";
/// The camera page, full-window, for the phone.
pub const CAMERA_PATH: &str = "/phone/camera";

pub async fn info(State(live): State<Live>) -> Response {
    match live.phone() {
        Some(p) => Json(Info {
            origin: &p.origin,
            setup_url: &p.setup_url,
            camera_url: format!("{}{CAMERA_PATH}", p.origin),
            ca_url: CA_PATH,
        })
        .into_response(),
        None => (StatusCode::NOT_FOUND, "no phone camera: this computer has no network address yet").into_response(),
    }
}

/// The setup page's address as a QR code, for the laptop's screen.
pub async fn qr(State(live): State<Live>) -> Response {
    use qrcode::render::svg;
    let Some(p) = live.phone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match qrcode::QrCode::new(&p.setup_url) {
        Ok(code) => {
            let image = code
                .render::<svg::Color>()
                .min_dimensions(220, 220)
                .dark_color(svg::Color("#0c131b"))
                .light_color(svg::Color("#ffffff"))
                .build();
            ([(header::CONTENT_TYPE, "image/svg+xml")], image).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// iOS Safari offers to install a profile for this content type.
pub async fn ca(State(live): State<Live>) -> Response {
    match live.phone() {
        Some(p) => ([(header::CONTENT_TYPE, "application/x-x509-ca-cert")], p.ca_der.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
