//! `/ws/live`: what the session is doing, for any number of screens (the
//! mirror on the front screen). The live loop publishes; every connected
//! screen gets each message. A screen that falls behind skips ahead rather
//! than queueing stale frames.

use analysis::cues::{Rhythm, Targets};
use analysis::ideal::{Guide, Sequence};
use analysis::pose::{Angles, Framing};
use analysis::technique::{Fault, Figure, Target};
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use serde::Serialize;
use tokio::sync::broadcast::{self, error::RecvError};

use crate::camera::Live;

/// Serialized messages, shared by every screen.
pub type LiveSender = broadcast::Sender<std::sync::Arc<str>>;

pub fn channel() -> LiveSender {
    broadcast::channel(256).0
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveMsg<'a> {
    /// Sent to each screen on connect: whether a session is running.
    Hello { live: bool },
    /// The body in one camera frame: thigh lengths from where the feet rest,
    /// x toward the feet, y up. With it, once the rower's proportions are
    /// known, the good stroke's body with the handle where theirs is.
    Figure { joints: &'a Figure, guide: Option<&'a Guide> },
    /// The good stroke on this rower's body: its catch and finish, and when
    /// each part moves in it. Every second, like the focus.
    Target(&'a Target),
    /// A completed stroke.
    Stroke(Box<StrokeMsg<'a>>),
    /// Something the coach said.
    Cue { text: &'a str },
    /// The focus fault for this session.
    Focus { fault: Option<Fault>, cue: Option<&'a str> },
    /// What this row aims for (each band `(low, high)`), and the stroke
    /// rhythm at the target rate. Every second, like the focus.
    Targets {
        #[serde(flatten)]
        targets: &'a Targets,
        rhythm: Option<Rhythm>,
    },
    /// The camera's setup check, twice a second while a camera page is connected.
    Framing(&'a Framing),
    /// The row `rowctl serve` runs, every second.
    Row(&'a crate::row::RowState),
}

#[derive(Debug, Serialize)]
pub struct StrokeMsg<'a> {
    pub index: u32,
    pub watts: Option<u16>,
    pub rate: Option<f64>,
    pub hr: Option<f64>,
    pub zone: Option<u8>,
    pub faults: Vec<FaultMsg>,
    /// The handle's drive and recovery, seconds, by the camera.
    pub drive_s: Option<f64>,
    pub recovery_s: Option<f64>,
    pub catch: Option<&'a Figure>,
    pub finish: Option<&'a Figure>,
    pub catch_angles: Option<Angles>,
    pub finish_angles: Option<Angles>,
    /// When the legs, the body and the arms each moved, by the handle.
    pub sequence: Option<Sequence>,
    pub typical_catch: &'a Figure,
    pub typical_finish: &'a Figure,
}

/// A fault a stroke showed, and what to do about it.
#[derive(Debug, Serialize)]
pub struct FaultMsg {
    pub fault: Fault,
    pub cue: &'static str,
}

impl From<Fault> for FaultMsg {
    fn from(fault: Fault) -> Self {
        Self { fault, cue: fault.cue() }
    }
}

/// Send to every screen (skipped when none is connected).
pub fn publish(tx: &LiveSender, msg: &LiveMsg) {
    if tx.receiver_count() == 0 {
        return;
    }
    if let Ok(text) = serde_json::to_string(msg) {
        let _ = tx.send(text.into());
    }
}

fn hello(live: bool) -> String {
    serde_json::to_string(&LiveMsg::Hello { live }).unwrap_or_default()
}

pub async fn upgrade(ws: WebSocketUpgrade, State(live): State<Live>) -> Response {
    ws.on_upgrade(move |socket| session(socket, live))
}

async fn session(mut socket: WebSocket, live: Live) {
    let rx = live.broadcast().map(|tx| tx.subscribe());
    if socket.send(Message::Text(hello(rx.is_some()).into())).await.is_err() {
        return;
    }
    let Some(mut rx) = rx else {
        // No session: stay connected quietly until the screen goes away.
        while let Some(Ok(m)) = socket.recv().await {
            if matches!(m, Message::Close(_)) {
                break;
            }
        }
        return;
    };
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(text) => {
                    if socket.send(Message::Text(text.to_string().into())).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => {
                    let _ = socket.send(Message::Text(hello(false).into())).await;
                    break;
                }
            },
            incoming = socket.recv() => match incoming {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => {}
            },
        }
    }
}
