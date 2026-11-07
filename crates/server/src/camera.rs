//! `/ws/camera`: the camera page streams pose landmarks in; the server stamps
//! them on the session clock, forwards them to the recorder (when there is
//! one), and answers with the framing check twice a second.
//!
//! Video stays on the phone. The exception is stills: small pictures with the
//! detected body drawn on, asked of the page one at a time. `GET
//! /api/camera/still.jpg` (this machine only) shows one for setting the
//! camera up, kept in memory until the next; `rowctl serve` also saves one
//! every so often during a row, beside its recording, to check the detected
//! body against the real one.

use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use analysis::pose::{Framing, framing};
use anyhow::{Result, bail};
use axum::body::Bytes;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use model::{Clock, Device, Event, EventKind, Link, POSE_JOINTS, Pose};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::sync::watch;
use tracing::{debug, info, warn};

/// Frames the framing check looks back over.
const WINDOW_US: i64 = 1_000_000;
const FRAMING_EVERY: Duration = Duration::from_millis(500);
/// How long the page asking for a still waits for the phone.
const STILL_WAIT: Duration = Duration::from_secs(3);
/// Bigger than this isn't a still (a 640-pixel JPEG is about 50 KB).
const STILL_MAX: usize = 1 << 20;

/// Stills of what the camera sees, for setting it up.
#[derive(Clone)]
struct Stills {
    /// Counts requests; the camera page's session sees each one.
    asked: watch::Sender<u64>,
    /// The latest still, with the request count it answered.
    latest: watch::Sender<Option<(u64, Bytes)>>,
}

impl Default for Stills {
    fn default() -> Self {
        Self {
            asked: watch::channel(0).0,
            latest: watch::channel(None).0,
        }
    }
}

/// Where camera frames go.
#[derive(Clone)]
pub struct Live {
    clock: Clock,
    tx: Option<mpsc::Sender<Event>>,
    /// The newest camera page; an older one stops when it sees a newer one.
    current: Arc<AtomicU64>,
    stills: Stills,
    broadcast: Option<crate::live::LiveSender>,
    /// Set once the machine has a network address, and replaced when it
    /// changes (the server runs for days; Wi-Fi comes and goes).
    phone: Arc<RwLock<Option<Arc<crate::phone::Phone>>>>,
}

impl Live {
    /// Framing check only: nothing is recorded (`rowctl serve`).
    pub fn setup_only(clock: Clock) -> Self {
        Self {
            clock,
            tx: None,
            current: Arc::default(),
            stills: Stills::default(),
            broadcast: None,
            phone: Arc::default(),
        }
    }

    /// Frames become events on `tx`, stamped with the session's `clock`.
    pub fn into_session(clock: Clock, tx: mpsc::Sender<Event>) -> Self {
        Self {
            clock,
            tx: Some(tx),
            current: Arc::default(),
            stills: Stills::default(),
            broadcast: None,
            phone: Arc::default(),
        }
    }

    /// Screens on `/ws/live` get what's published on `tx`.
    pub fn with_broadcast(self, tx: crate::live::LiveSender) -> Self {
        Self {
            broadcast: Some(tx),
            ..self
        }
    }

    pub fn broadcast(&self) -> Option<&crate::live::LiveSender> {
        self.broadcast.as_ref()
    }

    /// The camera page is also served over HTTPS for a phone.
    pub fn with_phone(self, phone: crate::phone::Phone) -> Self {
        self.set_phone(Some(phone));
        self
    }

    /// Shared by every clone: the setup page sees the change at once.
    pub fn set_phone(&self, phone: Option<crate::phone::Phone>) {
        if let Ok(mut slot) = self.phone.write() {
            *slot = phone.map(Arc::new);
        }
    }

    pub fn phone(&self) -> Option<Arc<crate::phone::Phone>> {
        self.phone.read().ok().and_then(|p| p.clone())
    }
}

/// One frame from the page. `p` is empty when no one is detected.
#[derive(Debug, Deserialize)]
struct FrameMsg {
    /// Capture time, µs since the Unix epoch, on the session clock (the page
    /// corrects its own clock with [`Ping`]s).
    t: f64,
    w: u16,
    h: u16,
    p: Vec<[f32; 3]>,
}

/// The page's clock reading, µs. The reply carries the session clock's, so
/// the page can work out its offset (it may be a phone with its own clock).
#[derive(Debug, Deserialize)]
struct Ping {
    ping: f64,
}

/// How the page is getting on (loading, the camera, errors): a phone has no
/// console, so it says so here and it lands in the log.
#[derive(Deserialize)]
struct Note {
    note: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Reply<'a> {
    /// Sent on connect, and again if the recording ends.
    Hello { recording: bool },
    Framing(&'a Framing),
    Clock { ping: f64, server_us: i64 },
    Error { message: String },
    /// Send a still of what the camera sees.
    Still,
}

fn parse_frame(text: &str) -> Result<Pose> {
    let m: FrameMsg = serde_json::from_str(text)?;
    if !m.p.is_empty() && m.p.len() != 2 * POSE_JOINTS.len() {
        bail!("expected {} points, got {}", 2 * POSE_JOINTS.len(), m.p.len());
    }
    if !m.t.is_finite() || m.p.iter().flatten().any(|v| !v.is_finite()) {
        bail!("non-finite value");
    }
    Ok(Pose {
        captured_us: m.t as i64,
        width: m.w,
        height: m.h,
        points: m.p,
    })
}

impl Live {
    /// A still of what the camera sees now, as a JPEG; `None` when no camera
    /// page answers within a few seconds.
    pub async fn still(&self) -> Option<Bytes> {
        let mut latest = self.stills.latest.subscribe();
        let mut n = 0;
        self.stills.asked.send_modify(|asked| {
            *asked += 1;
            n = *asked;
        });
        let answered = tokio::time::timeout(
            STILL_WAIT,
            latest.wait_for(|s| s.as_ref().is_some_and(|(k, _)| *k >= n)),
        )
        .await;
        match answered {
            Ok(Ok(s)) => s.as_ref().map(|(_, b)| b.clone()),
            _ => None,
        }
    }
}

/// `GET /api/camera/still.jpg`: what the camera sees now, from the phone.
pub async fn still(State(live): State<Live>) -> Response {
    match live.still().await {
        Some(jpeg) => ([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, "no-store")], jpeg).into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "The camera page didn't answer: is it open, and in front, on the phone?",
        )
            .into_response(),
    }
}

pub async fn upgrade(ws: WebSocketUpgrade, State(live): State<Live>) -> Response {
    ws.on_upgrade(move |socket| session(socket, live))
}

async fn send(socket: &mut WebSocket, reply: &Reply<'_>) -> bool {
    match serde_json::to_string(reply) {
        Ok(text) => socket.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => false,
    }
}

/// One camera page at a time, and the newest wins: it's the one in front of
/// the rower (a reload, or the phone after the laptop's own camera page). The
/// older one is told and stops, so the two don't take turns.
async fn session(mut socket: WebSocket, live: Live) {
    let me = live.current.fetch_add(1, Ordering::SeqCst) + 1;
    let replaced = || live.current.load(Ordering::SeqCst) != me;
    let mut tx = live.tx.clone();
    let link = |connected: bool| Event {
        t_us: live.clock.now_us(),
        kind: EventKind::Link(Link {
            device: Device::Camera,
            connected,
            detail: Some("browser".into()),
        }),
    };
    if let Some(t) = &tx {
        let _ = t.send(link(true)).await;
    }
    info!("camera page connected");
    if !send(&mut socket, &Reply::Hello { recording: tx.is_some() }).await {
        return;
    }

    let mut asked = live.stills.asked.subscribe();
    asked.mark_unchanged();
    let mut window: VecDeque<Pose> = VecDeque::new();
    let mut tick = tokio::time::interval(FRAMING_EVERY);
    let (mut dropped, mut bad) = (0u64, 0u64);
    loop {
        if replaced() {
            let message = "The camera page was opened again somewhere else, so this one stopped. Reload it to use this one.".to_string();
            send(&mut socket, &Reply::Error { message }).await;
            break;
        }
        tokio::select! {
            msg = socket.recv() => match msg {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(Ping { ping }) = serde_json::from_str::<Ping>(&text) {
                        let reply = Reply::Clock { ping, server_us: live.clock.now_us() };
                        if !send(&mut socket, &reply).await {
                            break;
                        }
                        continue;
                    }
                    if let Ok(Note { note }) = serde_json::from_str::<Note>(&text) {
                        info!("camera page: {}", note.chars().take(300).collect::<String>());
                        continue;
                    }
                    let pose = match parse_frame(&text) {
                        Ok(p) => p,
                        Err(e) => {
                            if bad == 0 {
                                warn!("camera: bad frame ({e:#}); further ones are counted, not logged");
                            }
                            bad += 1;
                            continue;
                        }
                    };
                    let newest = pose.captured_us;
                    window.push_back(pose.clone());
                    while window.front().is_some_and(|f| newest - f.captured_us > WINDOW_US) {
                        window.pop_front();
                    }
                    if let Some(t) = &tx {
                        let ev = Event { t_us: live.clock.now_us(), kind: EventKind::Pose(pose) };
                        match t.try_send(ev) {
                            Ok(()) => {}
                            Err(TrySendError::Full(_)) => dropped += 1,
                            Err(TrySendError::Closed(_)) => {
                                tx = None;
                                if !send(&mut socket, &Reply::Hello { recording: false }).await {
                                    break;
                                }
                            }
                        }
                    }
                }
                Some(Ok(Message::Binary(still))) => {
                    if still.len() <= STILL_MAX && still.starts_with(&[0xFF, 0xD8]) {
                        let n = *live.stills.asked.borrow();
                        live.stills.latest.send_replace(Some((n, still)));
                    }
                }
                Some(Ok(Message::Close(c))) => {
                    debug!("camera page closed the socket: {c:?}");
                    break;
                }
                Some(Err(e)) => {
                    debug!("camera page socket: {e}");
                    break;
                }
                None => break,
                Some(Ok(_)) => {}
            },
            Ok(()) = asked.changed() => {
                if !send(&mut socket, &Reply::Still).await {
                    break;
                }
            }
            _ = tick.tick() => {
                let f = framing(window.make_contiguous());
                if let Some(b) = &live.broadcast {
                    crate::live::publish(b, &crate::live::LiveMsg::Framing(&f));
                }
                if !send(&mut socket, &Reply::Framing(&f)).await {
                    break;
                }
            }
        }
    }
    // A newer page already said the camera is connected.
    if let Some(t) = &tx
        && !replaced()
    {
        let _ = t.send(link(false)).await;
    }
    info!("camera page disconnected ({dropped} frames dropped, {bad} malformed)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_parse_and_validate() {
        let pts = vec![[0.5f32, 0.5, 0.9]; 18];
        let ok = serde_json::json!({"t": 1.793e15, "w": 1280, "h": 720, "p": pts}).to_string();
        let pose = parse_frame(&ok).unwrap();
        assert_eq!((pose.captured_us, pose.width, pose.points.len()), (1_793_000_000_000_000, 1280, 18));
        let empty = r#"{"t": 1, "w": 640, "h": 480, "p": []}"#;
        assert!(parse_frame(empty).unwrap().points.is_empty());
        assert!(parse_frame(r#"{"t": 1, "w": 640, "h": 480, "p": [[0, 0, 1]]}"#).is_err());
        assert!(parse_frame("not json").is_err());
    }
}
