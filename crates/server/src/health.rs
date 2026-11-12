//! `/api/health`: readiness from the Apple Watch's days, and how those days
//! get here. The phone posts them to the home server (nutridag), which is
//! always on; this app, on a computer that sleeps, fetches the rower's own
//! days from there (`sync`, `keep_synced`). Nothing is accepted from the
//! network.

use std::sync::Arc;
use std::time::Duration;

use analysis::readiness::{HealthDay, Readiness, readiness, zone_rest};
use anyhow::{Context, Result, anyhow, bail};
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{Request, StatusCode, Uri, header};
use http_body_util::{BodyExt, Empty, Limited};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use store::SqlitePool;
use store::health::{day_number, list_days, parse_days, upsert_days};
use tokio::sync::Notify;
use tracing::{info, warn};

use crate::{Api, ApiError};

/// Days the readiness baseline looks back over, plus the week judged.
const LOOKBACK_DAYS: i64 = analysis::readiness::HRV_BASELINE_DAYS + analysis::readiness::WEEK;
/// The phone posts once a day; asking this often puts a Shortcut tapped by
/// hand on the row screen within ten minutes.
const FETCH_EVERY: Duration = Duration::from_secs(10 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// Three months of days are a few kilobytes.
const MAX_ANSWER_BYTES: usize = 1 << 20;

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

pub async fn get(State(api): State<Api>) -> Result<Json<HealthView>, ApiError> {
    let today = today();
    let days = list_days(&api.pool, today - LOOKBACK_DAYS).await?;
    Ok(Json(HealthView {
        readiness: readiness(&days, today),
        zone_rest: zone_rest(&days, today),
        days,
    }))
}

/// The rower's days as the home server has them: `GET <link>/days`. The link
/// is a credential, so no error here quotes it: only the server's name.
async fn fetch(link: &str) -> Result<Vec<HealthDay>> {
    let uri: Uri = format!("{link}/days").parse().map_err(|_| anyhow!("the stored link is not an address"))?;
    let host = uri.host().context("the stored link names no server")?.to_string();
    let at = format!("{host}:{}", uri.port_u16().unwrap_or(80));
    let stream = tokio::net::TcpStream::connect(&at).await.with_context(|| format!("connect to {at}"))?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .with_context(|| format!("talk to {at}"))?;
    // The connection ends with the one request it carries.
    tokio::spawn(connection);
    let request = Request::get(uri.path())
        .header(header::HOST, &at)
        .body(Empty::<Bytes>::new())?;
    let answer = sender.send_request(request).await.with_context(|| format!("ask {at}"))?;
    match answer.status() {
        StatusCode::OK => {}
        StatusCode::NOT_FOUND => bail!("{at} doesn't know the stored link: set the current one with `rowctl health --source`"),
        other => bail!("{at} answered {other}"),
    }
    let body = Limited::new(answer.into_body(), MAX_ANSWER_BYTES)
        .collect()
        .await
        .map_err(|e| anyhow!("read {at}'s answer: {e}"))?
        .to_bytes();
    parse_days(&body).with_context(|| format!("{at}'s answer"))
}

/// What one fetch brought.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Synced {
    /// Days the home server sent.
    pub days: usize,
    /// Whether any of them was new or different here.
    pub changed: bool,
}

/// Fetch the home server's days and store them over ours.
pub async fn sync(pool: &SqlitePool, link: &str) -> Result<Synced> {
    let days = tokio::time::timeout(FETCH_TIMEOUT, fetch(link))
        .await
        .map_err(|_| anyhow!("the home server didn't answer in {} s", FETCH_TIMEOUT.as_secs()))??;
    let Some(from) = days.iter().map(|d| d.day).min() else {
        return Ok(Synced { days: 0, changed: false });
    };
    let before = list_days(pool, from).await?;
    upsert_days(pool, &days).await?;
    Ok(Synced {
        days: days.len(),
        changed: list_days(pool, from).await? != before,
    })
}

/// Fetch now and every [`FETCH_EVERY`] for as long as the app runs, waking
/// `changed` when a fetch brought something new (the row runner re-plans).
/// A home server that can't be reached is logged when it starts failing and
/// when it answers again, not on every try; the stored days stay in use, and
/// readiness says so itself once they are stale.
pub async fn keep_synced(pool: SqlitePool, link: String, changed: Arc<Notify>) {
    let mut tick = tokio::time::interval(FETCH_EVERY);
    let mut failing = false;
    loop {
        tick.tick().await;
        match sync(&pool, &link).await {
            Ok(synced) => {
                if failing {
                    info!("Watch data: the home server answers again");
                }
                failing = false;
                if synced.changed {
                    info!("Watch data: new days from the home server");
                    changed.notify_one();
                }
            }
            Err(e) => {
                if !failing {
                    warn!("Watch data not fetched: {e:#}");
                }
                failing = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::extract::Path;
    use axum::routing;

    use super::*;

    const SECRET: &str = "s3cret-s3cret-s3cret";

    /// A home server with one known link and two days behind it.
    async fn home_server() -> String {
        let days = |Path(secret): Path<String>| async move {
            if secret != SECRET {
                return (StatusCode::NOT_FOUND, "Unknown link\n".to_string());
            }
            let today = chrono::Local::now().date_naive();
            let body = serde_json::json!({"days": [
                {"day": today.pred_opt().unwrap().to_string(), "hrv": 61.5, "resting_hr": 54.0, "steps": 9000.0},
                {"day": today.to_string(), "sleep_hours": 7.25},
            ]});
            (StatusCode::OK, body.to_string())
        };
        let app = Router::new().route("/health/{secret}/days", routing::get(days));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/health")
    }

    #[tokio::test]
    async fn the_home_servers_days_are_fetched_and_stored() {
        let dir = std::env::temp_dir().join(format!("row-assist-fetch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pool = store::db::open(&dir.join("t.db")).await.unwrap();
        let server = home_server().await;
        let link = format!("{server}/{SECRET}");

        assert_eq!(sync(&pool, &link).await.unwrap(), Synced { days: 2, changed: true });
        assert_eq!(sync(&pool, &link).await.unwrap(), Synced { days: 2, changed: false }, "nothing new the second time");
        let Json(view) = get(State(Api::new(pool.clone()))).await.ok().unwrap();
        assert_eq!((view.days[0].hrv_ms, view.days[0].resting_hr), (Some(61.5), Some(54.0)));
        assert_eq!(view.days[1].sleep_h, Some(7.25));

        // A link the server doesn't know says what to do, and never quotes itself.
        let stale = format!("{server}/another-secret-entirely");
        let e = format!("{:#}", sync(&pool, &stale).await.unwrap_err());
        assert!(e.contains("doesn't know the stored link") && !e.contains("another-secret"), "{e}");
        // So does a server that isn't there.
        let gone = format!("http://127.0.0.1:1/health/{SECRET}");
        let e = format!("{:#}", sync(&pool, &gone).await.unwrap_err());
        assert!(e.contains("connect to 127.0.0.1:1") && !e.contains(SECRET), "{e}");
        pool.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
