//! `/ws/camera`: the camera page streams pose landmarks in; the server stamps
//! them on the session clock, forwards them to the recorder (when there is
//! one), and answers with the framing check twice a second.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use analysis::pose::{Framing, framing};
use anyhow::{Result, bail};
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use model::{Clock, Device, Event, EventKind, Link, POSE_JOINTS, Pose};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, error::TrySendError};
use tracing::{info, warn};

/// Frames the framing check looks back over.
const WINDOW_US: i64 = 1_000_000;
const FRAMING_EVERY: Duration = Duration::from_millis(500);

/// Where camera frames go.
#[derive(Clone)]
pub struct Live {
    clock: Clock,
    tx: Option<mpsc::Sender<Event>>,
    busy: Arc<AtomicBool>,
    broadcast: Option<crate::live::LiveSender>,
    phone: Option<Arc<crate::phone::Phone>>,
}

impl Live {
    /// Framing check only: nothing is recorded (`rowctl serve`).
    pub fn setup_only(clock: Clock) -> Self {
        Self {
            clock,
            tx: None,
            busy: Arc::default(),
            broadcast: None,
            phone: None,
        }
    }

    /// Frames become events on `tx`, stamped with the session's `clock`.
    pub fn into_session(clock: Clock, tx: mpsc::Sender<Event>) -> Self {
        Self {
            clock,
            tx: Some(tx),
            busy: Arc::default(),
            broadcast: None,
            phone: None,
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
        Self {
            phone: Some(Arc::new(phone)),
            ..self
        }
    }

    pub fn phone(&self) -> Option<&crate::phone::Phone> {
        self.phone.as_deref()
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

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Reply<'a> {
    /// Sent on connect, and again if the recording ends.
    Hello { recording: bool },
    Framing(&'a Framing),
    Clock { ping: f64, server_us: i64 },
    Error { message: String },
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

pub async fn upgrade(ws: WebSocketUpgrade, State(live): State<Live>) -> Response {
    ws.on_upgrade(move |socket| session(socket, live))
}

/// Frees the camera slot however the session ends.
struct Slot(Arc<AtomicBool>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

async fn send(socket: &mut WebSocket, reply: &Reply<'_>) -> bool {
    match serde_json::to_string(reply) {
        Ok(text) => socket.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => false,
    }
}

async fn session(mut socket: WebSocket, live: Live) {
    if live.busy.swap(true, Ordering::SeqCst) {
        let message = "Another camera page is already connected; close it first".to_string();
        send(&mut socket, &Reply::Error { message }).await;
        return;
    }
    let _slot = Slot(live.busy.clone());
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

    let mut window: VecDeque<Pose> = VecDeque::new();
    let mut tick = tokio::time::interval(FRAMING_EVERY);
    let (mut dropped, mut bad) = (0u64, 0u64);
    loop {
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
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
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
    if let Some(t) = &tx {
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
