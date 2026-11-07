//! `/api/health`: Apple Watch recovery data in (from an iPhone Shortcut,
//! guarded by a token), readiness out.

use analysis::readiness::{HealthDay, Readiness, readiness, zone_rest};
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use serde::Serialize;
use store::health::{Samples, aggregate, day_number, list_days, upsert_days};
use tracing::warn;

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

/// What each token header carried, tidied: a phone capitalises the first
/// letter of a field and pastes stray spaces or quotes, none of which make it
/// a different token (it's lowercase hex).
fn presented(headers: &HeaderMap) -> Vec<(&'static str, String)> {
    let text = |name: &str| headers.get(name).map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
    let mut found = Vec::new();
    if let Some(v) = text("authorization") {
        let v = v.trim();
        let v = match v.get(..6) {
            Some(p) if p.eq_ignore_ascii_case("bearer") => &v[6..],
            _ => v,
        };
        found.push(("Authorization", tidy(v)));
    }
    if let Some(v) = text("x-row-assist-token") {
        found.push(("X-Row-Assist-Token", tidy(&v)));
    }
    found
}

fn tidy(t: &str) -> String {
    t.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '“' | '”' | '‘' | '’'))
        .collect::<String>()
        .to_lowercase()
}

/// Why no header matched, without saying anything about the token itself.
fn refusal(found: &[(&str, String)], want: usize) -> String {
    if found.is_empty() {
        return "no token sent: add a header X-Row-Assist-Token with the token as its value".into();
    }
    let each: Vec<String> = found
        .iter()
        .map(|(name, t)| match t.chars().count() {
            0 => format!("{name} is empty"),
            n if n != want => format!("{name} has {n} characters, the token has {want}"),
            _ => format!("{name} is the right length but a different token"),
        })
        .collect();
    each.join("; ")
}

/// Constant-time comparison: the token is a secret.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub async fn post(State(api): State<Api>, headers: HeaderMap, body: Bytes) -> Result<Json<HealthView>, ApiError> {
    let Some(expected) = api.token.as_deref() else {
        return Err(ApiError::Unauthorized("this computer has no token set up".into()));
    };
    let want = tidy(expected);
    let found = presented(&headers);
    if !found.iter().any(|(_, t)| same(t, &want)) {
        let why = refusal(&found, want.chars().count());
        warn!("Watch upload refused: {why}");
        return Err(ApiError::Unauthorized(why));
    }
    let unreadable = |e: String| {
        // The error can quote the whole upload: the start says what's wrong.
        let e: String = e.chars().take(300).collect();
        warn!("Watch upload unreadable: {e}");
        ApiError::BadRequest(e)
    };
    let samples: Samples = serde_json::from_slice(&body).map_err(|e| unreadable(format!("{e}")))?;
    let days = aggregate(&samples).map_err(|e| unreadable(format!("{e:#}")))?;
    upsert_days(&api.pool, &days).await?;
    if let Some(n) = &api.health_posted {
        n.notify_one();
    }
    Ok(Json(view(&api).await?))
}
