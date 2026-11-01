//! `/api/today`: what to row today, the command to row it, and why.

use analysis::planner::{Proposal, propose};
use analysis::readiness::{HRV_BASELINE_DAYS, Readiness, WEEK, readiness};
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use store::health::{day_number, list_days};

use crate::{Api, ApiError};

#[derive(Debug, Serialize)]
pub struct Today {
    pub proposal: Proposal,
    pub command: String,
    pub pm5_setup: Option<&'static str>,
    pub readiness: Readiness,
}

pub async fn get(State(api): State<Api>) -> Result<Json<Today>, ApiError> {
    let pool = &api.pool;
    let today = day_number(chrono::Local::now().date_naive());
    let health = list_days(pool, today - HRV_BASELINE_DAYS - WEEK).await?;
    let ready = readiness(&health, today);
    // From the browser, assume the camera will be on: it's set up there.
    let focus = store::history::auto_focus(pool, true).await?.map(|f| f.0);
    let rows = store::history::planner_rows(pool).await?;
    let proposal = propose(&rows, today, ready.level, focus);
    Ok(Json(Today {
        command: proposal.command(),
        pm5_setup: proposal.pm5_setup(),
        proposal,
        readiness: ready,
    }))
}
