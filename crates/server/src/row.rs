//! `/api/row`: the row `rowctl serve` runs. The server only reports it and
//! passes on the page's Stop and marks; the devices and the recorder are
//! `rowctl`'s.
//!
//! The same state is pushed to `/ws/live` every second as `{"type":"row",…}`.

use analysis::planner::Proposal;
use analysis::readiness::Readiness;
use analysis::technique::Fault;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};

use crate::Api;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Waiting for the first stroke: recording starts there.
    #[default]
    Ready,
    Rowing,
    /// Analysing and saving the row that just ended.
    Saving,
}

/// `connected` is `None` until the device has reported at all.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DeviceLink {
    pub connected: Option<bool>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Devices {
    pub pm5: DeviceLink,
    pub h10: DeviceLink,
    pub camera: DeviceLink,
}

/// What the next row is set up as.
#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub proposal: Proposal,
    pub readiness: Readiness,
    pub pm5_setup: Option<&'static str>,
    /// What the coach will call, e.g. "rate 19–21, 48–58 W, 20 min".
    pub cues: String,
}

/// How the last row ended.
#[derive(Debug, Clone, Serialize)]
pub struct Ended {
    pub at_us: i64,
    pub strokes: u32,
    /// The saved session; `None` when it was too short to keep.
    pub session: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RowState {
    pub phase: Phase,
    pub plan: Option<Plan>,
    pub devices: Devices,
    pub hr: Option<u16>,
    pub strokes: u32,
    /// Seconds since the first stroke, while rowing.
    pub elapsed_s: Option<f64>,
    /// Seconds since the last stroke, while rowing.
    pub idle_s: Option<f64>,
    /// A row ends by itself after this long without a stroke.
    pub ends_after_s: f64,
    pub last: Option<Ended>,
    /// When Watch data last arrived (µs), since the server started.
    pub watch_at_us: Option<i64>,
    /// What's being rowed on purpose from the last mark on, this row.
    pub mark: Option<Mark>,
}

/// What the rower is doing on purpose from now on, for checking the fault
/// detectors against (`rowctl check`): clean rowing, or one fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub fault: Option<Fault>,
}

impl Mark {
    /// The recording's marker label, as `rowctl check` reads it.
    pub fn label(self) -> &'static str {
        self.fault.map_or(analysis::fixtures::CLEAN, Fault::label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// End the row now (saved if it's long enough).
    Stop,
    Mark(Mark),
}

/// The server's end of the runner: its state, and a way to send it commands.
#[derive(Clone)]
pub struct Control {
    pub state: watch::Receiver<RowState>,
    pub commands: mpsc::Sender<Command>,
}

pub async fn get(State(api): State<Api>) -> Response {
    match &api.row {
        Some(c) => Json(c.state.borrow().clone()).into_response(),
        None => (StatusCode::NOT_FOUND, "no row running: start Row Assist with rowctl serve").into_response(),
    }
}

async fn command(api: &Api, c: Command) -> StatusCode {
    match &api.row {
        Some(row) if row.commands.send(c).await.is_ok() => StatusCode::ACCEPTED,
        Some(_) => StatusCode::SERVICE_UNAVAILABLE,
        None => StatusCode::NOT_FOUND,
    }
}

pub async fn stop(State(api): State<Api>) -> StatusCode {
    command(&api, Command::Stop).await
}

/// `POST /api/row/mark` with `{"fault": "early_arms"}`, or `{"fault": null}`
/// for clean rowing.
pub async fn mark(State(api): State<Api>, Json(m): Json<Mark>) -> StatusCode {
    command(&api, Command::Mark(m)).await
}
