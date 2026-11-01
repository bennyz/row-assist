//! `/api/health`: Apple Watch recovery data in (from an iPhone Shortcut,
//! guarded by a token), readiness out.

use analysis::readiness::{HealthDay, Readiness, readiness, zone_rest};
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use serde::Serialize;
use store::health::{Samples, aggregate, day_number, list_days, upsert_days};

use crate::{Api, ApiError};

/// Days the readiness baseline looks back over, plus the week judged.
const LOOKBACK_DAYS: i64 = analysis::readiness::HRV_BASELINE_DAYS + analysis::readiness::WEEK;

fn today() -> i64 {
    day_number(chrono::Local::now().date_naive())
}

#[derive(Debug, Serialize)]
pub struct HealthView {
    pub days: Vec<HealthDay>,
    pub readiness: Readiness,
    /// HRrest the zones use when none is set.
    pub zone_rest: Option<f64>,
}

async fn view(api: &Api) -> Result<HealthView, ApiError> {
    let today = today();
    let days = list_days(&api.pool, today - LOOKBACK_DAYS).await?;
    Ok(HealthView {
        readiness: readiness(&days, today),
        zone_rest: zone_rest(&days, today),
        days,
    })
}

pub async fn get(State(api): State<Api>) -> Result<Json<HealthView>, ApiError> {
    Ok(Json(view(&api).await?))
}

/// The token, from `Authorization: Bearer …` or `X-Row-Assist-Token`.
fn presented(headers: &HeaderMap) -> Option<&str> {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    bearer.or_else(|| headers.get("x-row-assist-token").and_then(|v| v.to_str().ok()))
}

/// Constant-time comparison: the token is a secret.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub async fn post(State(api): State<Api>, headers: HeaderMap, body: Bytes) -> Result<Json<HealthView>, ApiError> {
    let Some(expected) = api.token.as_deref() else {
        return Err(ApiError::Unauthorized);
    };
    if !presented(&headers).is_some_and(|t| same(t.trim(), expected)) {
        return Err(ApiError::Unauthorized);
    }
    let samples: Samples = serde_json::from_slice(&body).map_err(|e| ApiError::BadRequest(format!("{e}")))?;
    let days = aggregate(&samples).map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    upsert_days(&api.pool, &days).await?;
    Ok(Json(view(&api).await?))
}
