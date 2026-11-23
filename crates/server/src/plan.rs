//! `/api/plan`: where the plan stands, the rows ahead if each is rowed as
//! proposed, and the interval workouts' power from last time.

use analysis::planner::{MIN_ROW_MINUTES, Proposal, outline};
use analysis::progress::{ACTIVE_MIN_TARGET, Movement, STRENGTH_TARGET, movement};
use analysis::readiness::{HRV_BASELINE_DAYS, Readiness, WEEK, readiness};
use analysis::workout::Kind;
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use store::health::{day_number, list_days};

use crate::{Api, ApiError};

/// Weeks the outline looks ahead.
pub const WEEKS_AHEAD: u32 = 4;

#[derive(Debug, Serialize)]
pub struct PlanView {
    /// Days since 1970-01-01, local.
    pub today: i64,
    pub readiness: Readiness,
    /// Rows of 10 minutes or more in the last 7 days.
    pub done: Vec<Done>,
    pub target_per_week: u32,
    /// The last 7 days' movement from the Watch, against a week's aims.
    pub movement: Movement,
    pub active_min_target: f64,
    pub strength_target: u32,
    /// Today's proposal first, then the ones after it.
    pub ahead: Vec<Ahead>,
    /// Each interval workout's power for next time, and why.
    pub power: Vec<Power>,
}

#[derive(Debug, Serialize)]
pub struct Done {
    pub day: i64,
    /// The name its recording keeps (`None` for the logbook's).
    pub plan: Option<String>,
    pub minutes: f64,
}

#[derive(Debug, Serialize)]
pub struct Ahead {
    pub day: i64,
    #[serde(flatten)]
    pub proposal: Proposal,
}

#[derive(Debug, Serialize)]
pub struct Power {
    pub kind: Kind,
    pub note: String,
}

pub async fn get(State(api): State<Api>) -> Result<Json<PlanView>, ApiError> {
    let pool = &api.pool;
    let today = day_number(chrono::Local::now().date_naive());
    let health = list_days(pool, today - HRV_BASELINE_DAYS - WEEK).await?;
    let ready = readiness(&health, today);
    // From the browser, assume the camera will be on: it's set up there.
    let focus = store::history::auto_focus(pool, true).await?.map(|f| f.0);
    let rows = store::history::planner_rows(pool).await?;
    let heart = store::history::heart_profile(pool, today).await?;
    let mut power = Vec::new();
    // An easy row's stands for the steady row's and the minimum session's too.
    for kind in Kind::ALL.into_iter().filter(|k| k.intervals().is_some() || *k == Kind::Easy) {
        if let Some(p) = store::history::progression(pool, kind, heart.as_ref()).await? {
            power.push(Power { kind, note: p.note });
        }
    }
    let mut done: Vec<Done> = rows
        .iter()
        .filter(|r| r.day > today - WEEK && r.day <= today && r.minutes >= MIN_ROW_MINUTES)
        .map(|r| Done {
            day: r.day,
            plan: r.plan.clone(),
            minutes: r.minutes,
        })
        .collect();
    done.sort_by_key(|d| d.day);
    let ahead: Vec<Ahead> = outline(&rows, today, ready.level, focus, WEEKS_AHEAD)
        .into_iter()
        .map(|(day, proposal)| Ahead { day, proposal })
        .collect();
    let week = health.iter().filter(|d| d.day > today - WEEK && d.day <= today);
    Ok(Json(PlanView {
        today,
        readiness: ready,
        done,
        target_per_week: ahead.first().map_or(analysis::planner::ROWS_PER_WEEK, |a| a.proposal.target_per_week),
        movement: movement(week),
        active_min_target: ACTIVE_MIN_TARGET,
        strength_target: STRENGTH_TARGET,
        ahead,
        power,
    }))
}
