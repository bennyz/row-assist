//! HTTP server: a JSON API over stored sessions, the camera WebSocket, and
//! the built frontend.
//!
//! The API hands the browser view-ready data (per-stroke rate and work, the
//! session's mean force curve, zone bands) so that interpretation stays in
//! `analysis` and the frontend only draws.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use analysis::session::{self, Summary};
use analysis::strokes::StrokeRecord;
use analysis::technique::{self, FaultHit, Figure, StrokePose};
use anyhow::{Context, Result};
use axum::Json;
use axum::Router;
use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;
use store::SqlitePool;
use store::sessions::SessionRow;
use tower_http::services::{ServeDir, ServeFile};
use tracing::error;

pub mod camera;
pub mod health;
pub mod live;
pub mod phone;
pub mod progress;
pub mod tls;
pub mod today;
pub use camera::Live;

pub struct Config {
    pub addr: SocketAddr,
    pub db: PathBuf,
    /// The built frontend (`frontend/dist`).
    pub web_dir: PathBuf,
    /// Accept Watch data posted with this token.
    pub token: Option<String>,
}

/// What the JSON API needs.
#[derive(Clone)]
pub struct Api {
    pub pool: SqlitePool,
    /// Posting Watch data needs it; `None` turns posting off.
    pub token: Option<String>,
}

impl From<SqlitePool> for Api {
    fn from(pool: SqlitePool) -> Self {
        Self { pool, token: None }
    }
}

/// Serve review and camera setup (nothing recorded) until Ctrl-C.
pub async fn serve(cfg: Config) -> Result<()> {
    let pool = store::db::open(&cfg.db).await?;
    let api = Api { pool, token: cfg.token };
    let app = router(Some(api), &cfg.web_dir, Live::setup_only(model::Clock::start()));
    let (addr, task) = listen(cfg.addr, app).await?;
    println!("Row Assist on http://{addr} (Ctrl-C to stop)");
    if !cfg.web_dir.join("index.html").is_file() {
        println!("Frontend not built ({}): only the API is up", cfg.web_dir.display());
    }
    let _ = tokio::signal::ctrl_c().await;
    task.abort();
    Ok(())
}

/// Bind and serve in the background. Returns the bound address (useful with port 0).
pub async fn listen(addr: SocketAddr, app: Router) -> Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("listen on {addr}"))?;
    let bound = listener.local_addr()?;
    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            error!("server: {e:#}");
        }
    });
    Ok((bound, task))
}

/// `api`: the JSON API, when a database is available.
pub fn router(api: Option<Api>, web_dir: &Path, live: Live) -> Router {
    let mut app = Router::new()
        .route("/ws/camera", get(camera::upgrade))
        .route("/ws/live", get(live::upgrade))
        .route("/api/phone", get(phone::info))
        .route(phone::CA_PATH, get(phone::ca))
        .with_state(live);
    if let Some(api) = api {
        app = app.merge(
            Router::new()
                .route("/api/sessions", get(list_sessions))
                .route("/api/sessions/{id}", get(session_detail))
                .route("/api/health", get(health::get))
                .route("/api/progress", get(progress::get))
                .route("/api/today", get(today::get))
                .route("/api/health/daily", axum::routing::post(health::post))
                .with_state(api),
        );
    }
    let index = web_dir.join("index.html");
    if index.is_file() {
        // Unknown paths get index.html, so the app's own routes survive a reload.
        app.fallback_service(ServeDir::new(web_dir).fallback(ServeFile::new(index)))
    } else {
        app.fallback(not_built)
    }
}

async fn not_built() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Html(
            "<!doctype html><title>Row Assist</title><body style=\"font-family:sans-serif;\
             background:#0d131a;color:#e6e1d6;padding:2rem\"><p>The frontend isn't built. \
             Run <code>npm install &amp;&amp; npm run build</code> in <code>frontend/</code>, \
             then reload.</p>",
        ),
    )
}

/// One row of the session list.
#[derive(Debug, Serialize)]
pub struct SessionItem {
    pub id: i64,
    pub started_at_us: i64,
    pub plan: Option<String>,
    pub note: Option<String>,
    pub simulated: bool,
    pub minutes: f64,
    pub strokes: u32,
    pub distance_m: Option<f64>,
    pub watts: Option<f64>,
    pub spm: Option<f64>,
    pub hr: Option<f64>,
    pub ef: Option<f64>,
}

impl From<&SessionRow> for SessionItem {
    fn from(r: &SessionRow) -> Self {
        let w = &r.summary.whole;
        Self {
            id: r.id,
            started_at_us: r.summary.started_at_us,
            plan: r.meta.plan.clone(),
            note: r.meta.note.clone(),
            simulated: r.meta.simulated,
            minutes: w.seconds / 60.0,
            strokes: w.strokes,
            distance_m: w.distance_m,
            watts: w.watts,
            spm: w.spm,
            hr: w.hr,
            ef: w.ef,
        }
    }
}

/// A stroke as the review screen draws it.
#[derive(Debug, Serialize)]
pub struct StrokeView {
    pub index: u32,
    /// Seconds from the session's first catch.
    pub t_s: f64,
    pub drive_ms: Option<f64>,
    pub recovery_ms: Option<f64>,
    pub ratio: Option<f64>,
    pub watts: Option<u16>,
    /// From the stroke's own timing; `None` for a stroke spanning a pause.
    pub rate: Option<f64>,
    pub joules: Option<f64>,
    pub distance_m: Option<f64>,
    pub hr: Option<f64>,
    pub peak_n: Option<f64>,
    pub drag_factor: Option<u8>,
    pub force_n: Vec<f64>,
    pub pose: Option<StrokePose>,
    pub faults: Vec<FaultHit>,
}

impl StrokeView {
    fn new(s: &StrokeRecord, start_us: i64) -> Self {
        let paused = s.cycle_s().is_some_and(|c| c > session::MAX_CYCLE_S);
        Self {
            index: s.index,
            t_s: (s.t_catch_us - start_us) as f64 / 1e6,
            drive_ms: s.drive_ms,
            recovery_ms: s.recovery_ms.filter(|_| !paused),
            ratio: s.ratio.filter(|_| !paused),
            watts: s.watts,
            rate: s.rate().filter(|_| !paused),
            joules: s.joules().filter(|_| !paused),
            distance_m: s.distance_m,
            hr: s.hr_bpm,
            peak_n: s.peak_n,
            drag_factor: s.drag_factor,
            force_n: s.force_n.clone(),
            pose: s.pose.clone(),
            faults: s.faults.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ZoneBand {
    pub zone: u8,
    pub lo: f64,
    pub hi: f64,
}

#[derive(Debug, Serialize)]
pub struct SessionDetail {
    #[serde(flatten)]
    pub item: SessionItem,
    pub recording: String,
    pub summary: Summary,
    pub strokes: Vec<StrokeView>,
    pub mean_curve: Vec<f64>,
    pub strongest: Option<u32>,
    pub weakest: Option<u32>,
    /// Bands of the heart profile the session was analysed with.
    pub zones: Vec<ZoneBand>,
    /// The session's median catch and finish figures (empty without a camera).
    pub typical_catch: Figure,
    pub typical_finish: Figure,
    /// EF against past steady pieces at similar power (steady rows of 15+ min).
    pub reference: Option<ReferenceView>,
}

#[derive(Debug, Serialize)]
pub struct ReferenceView {
    pub ef: Option<f64>,
    pub watts: Option<f64>,
    /// The power band past pieces were taken from.
    pub watts_band: Option<(f64, f64)>,
    /// Mean EF of the best and worst comparable pieces.
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub pieces: usize,
    /// The comeback-ramp verdict, for a reference row.
    pub ramp: Option<String>,
    /// Why there's no comparison, when there isn't.
    pub note: Option<String>,
}

async fn reference_view(pool: &SqlitePool, row: &SessionRow) -> anyhow::Result<Option<ReferenceView>> {
    use analysis::reference::{MIN_MINUTES, MIN_PIECES, Outcome, WATTS_WINDOW, compare};
    let band = |w: f64| Some((w - WATTS_WINDOW, w + WATTS_WINDOW));
    let is_reference = row.meta.plan.as_deref() == Some("reference");
    if !is_reference && row.summary.whole.seconds < MIN_MINUTES * 60.0 {
        return Ok(None);
    }
    let history = store::history::pieces(pool, row.summary.started_at_us).await?;
    Ok(match compare(&row.summary.whole, &history) {
        Outcome::Compared(c) => Some(ReferenceView {
            ef: Some(c.ef),
            watts: Some(c.watts),
            watts_band: band(c.watts),
            high: Some(c.high),
            low: Some(c.low),
            pieces: c.pieces.len(),
            ramp: is_reference.then(|| c.ramp.describe().to_string()),
            note: None,
        }),
        Outcome::NoBaseline { ef, watts, pieces } => Some(ReferenceView {
            ef: Some(ef),
            watts: Some(watts),
            watts_band: band(watts),
            high: None,
            low: None,
            pieces,
            ramp: None,
            note: Some(format!("only {pieces} past steady pieces near {watts:.0} W; {MIN_PIECES} needed to compare")),
        }),
        Outcome::NotSteady(why) if is_reference => Some(ReferenceView {
            ef: None,
            watts: None,
            watts_band: None,
            high: None,
            low: None,
            pieces: 0,
            ramp: None,
            note: Some(format!("not usable as a reference row: {why}")),
        }),
        Outcome::NotSteady(_) => None,
    })
}

async fn list_sessions(State(api): State<Api>) -> Result<Json<Vec<SessionItem>>, ApiError> {
    let rows = store::sessions::list_sessions(&api.pool).await?;
    // Newest first for the list.
    Ok(Json(rows.iter().rev().map(SessionItem::from).collect()))
}

async fn session_detail(
    State(api): State<Api>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<SessionDetail>, ApiError> {
    let pool = &api.pool;
    let row = store::sessions::get_session(pool, id).await?.ok_or(ApiError::NotFound)?;
    let strokes = store::sessions::session_strokes(pool, id).await?;
    let start = row.summary.started_at_us;
    let extremes = session::work_extremes(&strokes);
    let (typical_catch, typical_finish) = technique::typical_figures(&strokes);
    let reference = reference_view(pool, &row).await?;
    let zones = row
        .summary
        .profile
        .map(|p| {
            (1..=5)
                .filter_map(|z| p.zone_bpm(z).map(|(lo, hi)| ZoneBand { zone: z, lo, hi }))
                .collect()
        })
        .unwrap_or_default();
    Ok(Json(SessionDetail {
        item: SessionItem::from(&row),
        recording: row.meta.recording.clone(),
        mean_curve: session::mean_curve(&strokes),
        strongest: extremes.map(|e| e.0),
        weakest: extremes.map(|e| e.1),
        strokes: strokes.iter().map(|s| StrokeView::new(s, start)).collect(),
        zones,
        typical_catch,
        typical_finish,
        reference,
        summary: row.summary,
    }))
}

pub enum ApiError {
    NotFound,
    Unauthorized,
    BadRequest(String),
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "not found"}))).into_response(),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "missing or wrong token (see `rowctl health --token`)"})),
            )
                .into_response(),
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": m}))).into_response(),
            Self::Internal(e) => {
                error!("API: {e:#}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("{e:#}")})),
                )
                    .into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use analysis::session::Segment;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use serde_json::Value;
    use store::sessions::{SessionMeta, save_session};
    use tower::ServiceExt;

    async fn fixture(name: &str) -> (SqlitePool, PathBuf) {
        let dir = std::env::temp_dir().join(format!("row-assist-server-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pool = store::db::open(&dir.join("t.db")).await.unwrap();
        (pool, dir)
    }

    fn stroke(i: u32, start: i64) -> StrokeRecord {
        let t = start + i as i64 * 3_000_000;
        StrokeRecord {
            index: i,
            t_catch_us: t,
            t_finish_us: Some(t + 1_000_000),
            t_next_catch_us: Some(t + if i == 3 { 20_000_000 } else { 3_000_000 }),
            drive_ms: Some(1000.0),
            recovery_ms: Some(2000.0),
            ratio: Some(2.0),
            watts: Some(80),
            spm: Some(20),
            distance_m: Some(9.0),
            drag_factor: Some(110),
            hr_bpm: Some(140.0),
            peak_n: Some(500.0),
            force_n: vec![],
            pose: None,
            faults: vec![],
        }
    }

    async fn get(app: Router, uri: &str) -> (StatusCode, Value) {
        let res = app
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let body = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn lists_and_details_sessions() {
        let (pool, dir) = fixture("api").await;
        let start = 1_793_000_000_000_000;
        let summary = Summary {
            started_at_us: start,
            whole: Segment {
                strokes: 5,
                seconds: 900.0,
                watts: Some(80.0),
                ..Default::default()
            },
            ..Default::default()
        };
        let meta = SessionMeta {
            started_at_us: start - 5_000_000,
            recording: "/x.rowrec".into(),
            note: None,
            plan: Some("easy".into()),
            simulated: false,
        };
        let strokes: Vec<_> = (0..5).map(|i| stroke(i, start)).collect();
        let id = save_session(&pool, &meta, &summary, &strokes).await.unwrap();
        let app = router(Some(pool.into()), &dir.join("no-frontend"), Live::setup_only(model::Clock::start()));

        let (status, list) = get(app.clone(), "/api/sessions").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list[0]["id"], id);
        assert_eq!(list[0]["minutes"], 15.0);
        assert_eq!(list[0]["plan"], "easy");

        let (status, d) = get(app.clone(), &format!("/api/sessions/{id}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(d["strokes"].as_array().unwrap().len(), 5);
        assert_eq!(d["strokes"][1]["t_s"], 3.0);
        assert_eq!(d["strokes"][1]["rate"], 20.0);
        assert_eq!(d["strokes"][3]["rate"], Value::Null, "spans a 20 s pause");
        assert_eq!(d["recording"], "/x.rowrec");

        let (status, t) = get(app.clone(), "/api/today").await;
        assert_eq!(status, StatusCode::OK);
        assert!(t["command"].as_str().unwrap().starts_with("rowctl record --plan"));
        let (status, p) = get(app.clone(), "/api/progress").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(p["weeks"][0]["rows"], 1, "the one saved session");
        let (status, _) = get(app.clone(), "/api/sessions/999").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = get(app, "/review/1").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "no frontend: the hint page");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn watch_data_needs_the_token() {
        let (pool, dir) = fixture("health").await;
        let api = Api { pool, token: Some("s3cret-token-0123".into()) };
        let app = router(Some(api), &dir.join("no-frontend"), Live::setup_only(model::Clock::start()));
        let today = chrono::Local::now().date_naive();
        let body = serde_json::json!({
            "resting_hr": [{"date": today.to_string(), "value": 54}],
            "hrv": [{"date": format!("{today}T03:00:00+00:00"), "value": 61.5}],
        })
        .to_string();
        let post = |token: Option<&str>, body: String| {
            let mut req = Request::post("/api/health/daily").header("content-type", "application/json");
            if let Some(t) = token {
                req = req.header("authorization", format!("Bearer {t}"));
            }
            req.body(Body::from(body)).unwrap()
        };
        let status = |res: axum::response::Response| res.status();
        assert_eq!(status(app.clone().oneshot(post(None, body.clone())).await.unwrap()), StatusCode::UNAUTHORIZED);
        assert_eq!(status(app.clone().oneshot(post(Some("nope"), body.clone())).await.unwrap()), StatusCode::UNAUTHORIZED);
        assert_eq!(status(app.clone().oneshot(post(Some("s3cret-token-0123"), "{".into())).await.unwrap()), StatusCode::BAD_REQUEST);
        assert_eq!(status(app.clone().oneshot(post(Some("s3cret-token-0123"), body)).await.unwrap()), StatusCode::OK);

        let (s, health) = get(app, "/api/health").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(health["days"][0]["resting_hr"], 54.0);
        assert_eq!(health["days"][0]["hrv_ms"], 61.5);
        assert_eq!(health["readiness"]["level"], "unknown", "one day is no baseline");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn unknown_paths_get_the_app() {
        let (pool, dir) = fixture("spa").await;
        let web = dir.join("dist");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("index.html"), "<!doctype html><p>app</p>").unwrap();
        let app = router(Some(pool.into()), &web, Live::setup_only(model::Clock::start()));
        for uri in ["/", "/sessions/4"] {
            let res = app
                .clone()
                .oneshot(Request::get(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{uri}");
            let body = to_bytes(res.into_body(), 1 << 20).await.unwrap();
            assert!(body.starts_with(b"<!doctype html><p>app"), "{uri}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    mod ws {
        use futures::{SinkExt, StreamExt};
        use model::{Clock, Device, EventKind};
        use tokio::sync::mpsc;
        use tokio_tungstenite::connect_async;
        use tokio_tungstenite::tungstenite::Message;

        use super::super::*;

        async fn next_json(
            ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
        ) -> serde_json::Value {
            loop {
                match ws.next().await.unwrap().unwrap() {
                    Message::Text(t) => return serde_json::from_str(&t).unwrap(),
                    _ => continue,
                }
            }
        }

        fn frame(t_us: i64) -> String {
            // Seated side-on, left side to the camera (as the pose tests build it).
            let near: Vec<[f32; 3]> = [
                (560.0, 250.0), (560.0, 300.0), (640.0, 380.0), (720.0, 460.0), (480.0, 500.0),
                (700.0, 360.0), (700.0, 560.0), (680.0, 580.0), (760.0, 580.0),
            ]
            .iter()
            .map(|&(x, y): &(f32, f32)| [x / 1280.0, y / 720.0, 0.95])
            .collect();
            let far: Vec<[f32; 3]> = near.iter().map(|&[x, y, _]| [x + 0.008, y, 0.6]).collect();
            let points = [near, far].concat();
            serde_json::json!({"t": t_us, "w": 1280, "h": 720, "p": points}).to_string()
        }

        #[tokio::test]
        async fn every_screen_gets_the_live_messages() {
            let dir = std::env::temp_dir().join(format!("row-assist-live-{}", std::process::id()));
            let (tx, _rx) = mpsc::channel(16);
            let hub = crate::live::channel();
            let app = router(None, &dir, Live::into_session(Clock::start(), tx).with_broadcast(hub.clone()));
            let (addr, server) = listen("127.0.0.1:0".parse().unwrap(), app).await.unwrap();

            let (mut a, _) = connect_async(format!("ws://{addr}/ws/live")).await.unwrap();
            let (mut b, _) = connect_async(format!("ws://{addr}/ws/live")).await.unwrap();
            for ws in [&mut a, &mut b] {
                let hello = next_json(ws).await;
                assert_eq!((hello["type"].as_str(), hello["live"].as_bool()), (Some("hello"), Some(true)));
                assert_eq!(hello["catch_trunk"][0], 15.0);
            }
            crate::live::publish(&hub, &crate::live::LiveMsg::Cue { text: "Legs first, arms long" });
            for ws in [&mut a, &mut b] {
                let m = next_json(ws).await;
                assert_eq!((m["type"].as_str(), m["text"].as_str()), (Some("cue"), Some("Legs first, arms long")));
            }
            server.abort();
        }

        #[tokio::test]
        async fn without_a_session_screens_are_told_so() {
            let dir = std::env::temp_dir().join(format!("row-assist-nolive-{}", std::process::id()));
            let app = router(None, &dir, Live::setup_only(Clock::start()));
            let (addr, server) = listen("127.0.0.1:0".parse().unwrap(), app).await.unwrap();
            let (mut ws, _) = connect_async(format!("ws://{addr}/ws/live")).await.unwrap();
            assert_eq!(next_json(&mut ws).await["live"], false);
            server.abort();
        }

        #[tokio::test]
        async fn the_mirror_gets_the_setup_check() {
            let dir = std::env::temp_dir().join(format!("row-assist-setup-{}", std::process::id()));
            let (tx, _rx) = mpsc::channel(256);
            let hub = crate::live::channel();
            let clock = Clock::start();
            let app = router(None, &dir, Live::into_session(clock, tx).with_broadcast(hub));
            let (addr, server) = listen("127.0.0.1:0".parse().unwrap(), app).await.unwrap();

            let (mut screen, _) = connect_async(format!("ws://{addr}/ws/live")).await.unwrap();
            assert_eq!(next_json(&mut screen).await["type"], "hello");
            let (mut cam, _) = connect_async(format!("ws://{addr}/ws/camera")).await.unwrap();
            let t0 = clock.now_us();
            for i in 0..30 {
                cam.send(Message::Text(frame(t0 + i * 33_333).into())).await.unwrap();
            }
            let setup = loop {
                let m = next_json(&mut screen).await;
                if m["type"] == "framing" && m["frames"] == 30 {
                    break m;
                }
            };
            assert_eq!(setup["ok"], true, "{setup:#}");
            server.abort();
        }

        #[tokio::test]
        async fn phone_setup_is_served_only_with_a_phone() {
            let dir = std::env::temp_dir().join(format!("row-assist-phone-{}", std::process::id()));
            let get = |app: Router, uri: &'static str| async move {
                use tower::ServiceExt;
                let req = axum::http::Request::get(uri).body(axum::body::Body::empty()).unwrap();
                let res = app.oneshot(req).await.unwrap();
                let status = res.status();
                (status, axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap())
            };
            let none = router(None, &dir, Live::setup_only(Clock::start()));
            assert_eq!(get(none.clone(), "/api/phone").await.0, StatusCode::NOT_FOUND);
            assert_eq!(get(none, "/phone/ca.cer").await.0, StatusCode::NOT_FOUND);

            let phone = crate::phone::Phone { origin: "https://192.168.1.20:8443".into(), ca_der: vec![1, 2, 3] };
            let app = router(None, &dir, Live::setup_only(Clock::start()).with_phone(phone));
            let (status, body) = get(app.clone(), "/api/phone").await;
            assert_eq!(status, StatusCode::OK);
            let info: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(info["camera_url"], "https://192.168.1.20:8443/phone/camera");
            assert_eq!(info["origin"], "https://192.168.1.20:8443");
            assert_eq!(get(app, "/phone/ca.cer").await.1.as_ref(), &[1, 2, 3]);
        }

        #[tokio::test]
        async fn frames_are_recorded_and_framing_comes_back() {
            let dir = std::env::temp_dir().join(format!("row-assist-ws-{}", std::process::id()));
            let (tx, mut rx) = mpsc::channel(256);
            let clock = Clock::start();
            let app = router(None, &dir, Live::into_session(clock, tx));
            let (addr, server) = listen("127.0.0.1:0".parse().unwrap(), app).await.unwrap();

            let (mut ws, _) = connect_async(format!("ws://{addr}/ws/camera")).await.unwrap();
            assert_eq!(next_json(&mut ws).await["recording"], true);

            // A second page is turned away while the first is connected.
            let (mut other, _) = connect_async(format!("ws://{addr}/ws/camera")).await.unwrap();
            assert_eq!(next_json(&mut other).await["type"], "error");

            let t0 = clock.now_us();
            for i in 0..30 {
                ws.send(Message::Text(frame(t0 + i * 33_333).into())).await.unwrap();
            }
            ws.send(Message::Text("garbage".into())).await.unwrap();

            // Framing replies arrive every 500 ms; wait for one that has seen the frames.
            let framing = loop {
                let m = next_json(&mut ws).await;
                if m["type"] == "framing" && m["frames"] == 30 {
                    break m;
                }
            };
            assert_eq!(framing["side"], "left");
            assert_eq!(framing["ok"], true, "{framing:#}");

            ws.close(None).await.unwrap();
            let mut poses = 0;
            let mut links = vec![];
            while let Some(ev) = rx.recv().await {
                match ev.kind {
                    EventKind::Pose(p) => {
                        assert_eq!(p.points.len(), 18);
                        poses += 1;
                    }
                    EventKind::Link(l) => {
                        assert_eq!(l.device, Device::Camera);
                        links.push(l.connected);
                        if !l.connected {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            assert_eq!(poses, 30, "the garbage frame is skipped");
            assert_eq!(links, vec![true, false]);
            server.abort();
        }
    }
}