//! A phone as the side camera: `/api/phone` tells the setup page where the
//! camera page is served over HTTPS, and `/phone/ca.cer` is the certificate
//! the phone installs to trust it.

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::camera::Live;

/// Served with `rowctl record --phone`.
#[derive(Debug, Clone)]
pub struct Phone {
    /// The HTTPS origin, e.g. `https://192.168.1.20:8443`.
    pub origin: String,
    /// The CA certificate (DER).
    pub ca_der: Vec<u8>,
}

#[derive(Serialize)]
struct Info<'a> {
    origin: &'a str,
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
            camera_url: format!("{}{CAMERA_PATH}", p.origin),
            ca_url: CA_PATH,
        })
        .into_response(),
        None => (StatusCode::NOT_FOUND, "not serving a phone camera (rowctl record --phone)").into_response(),
    }
}

/// iOS Safari offers to install a profile for this content type.
pub async fn ca(State(live): State<Live>) -> Response {
    match live.phone() {
        Some(p) => ([(header::CONTENT_TYPE, "application/x-x509-ca-cert")], p.ca_der.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
