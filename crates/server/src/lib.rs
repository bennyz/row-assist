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
pub use camera::Live;

pub struct Config {
    pub addr: SocketAddr,
    pub db: PathBuf,
    /// The built frontend (`frontend/dist`).
    pub web_dir: PathBuf,
}

/// Serve review and camera setup (nothing recorded) until Ctrl-C.
pub async fn serve(cfg: Config) -> Result<()> {
    let pool = store::db::open(&cfg.db).await?;
    let app = router(Some(pool), &cfg.web_dir, Live::setup_only(model::Clock::start()));
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

/// `pool`: the session API, when a database is available.
pub fn router(pool: Option<SqlitePool>, web_dir: &Path, live: Live) -> Router {
    let mut app = Router::new()
        .route("/ws/camera", get(camera::upgrade))
        .with_state(live);
    if let Some(pool) = pool {
        app = app.merge(
            Router::new()
                .route("/api/sessions", get(list_sessions))
                .route("/api/sessions/{id}", get(session_detail))
                .with_state(pool),
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
}

async fn list_sessions(State(pool): State<SqlitePool>) -> Result<Json<Vec<SessionItem>>, ApiError> {
    let rows = store::sessions::list_sessions(&pool).await?;
    // Newest first for the list.
    Ok(Json(rows.iter().rev().map(SessionItem::from).collect()))
}

async fn session_detail(
    State(pool): State<SqlitePool>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<SessionDetail>, ApiError> {
    let row = store::sessions::get_session(&pool, id).await?.ok_or(ApiError::NotFound)?;
    let strokes = store::sessions::session_strokes(&pool, id).await?;
    let start = row.summary.started_at_us;
    let extremes = session::work_extremes(&strokes);
    let (typical_catch, typical_finish) = technique::typical_figures(&strokes);
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
        summary: row.summary,
    }))
}

pub enum ApiError {
    NotFound,
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
        let app = router(Some(pool), &dir.join("no-frontend"), Live::setup_only(model::Clock::start()));

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

        let (status, _) = get(app.clone(), "/api/sessions/999").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = get(app, "/review/1").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "no frontend: the hint page");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn unknown_paths_get_the_app() {
        let (pool, dir) = fixture("spa").await;
        let web = dir.join("dist");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("index.html"), "<!doctype html><p>app</p>").unwrap();
        let app = router(Some(pool), &web, Live::setup_only(model::Clock::start()));
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
