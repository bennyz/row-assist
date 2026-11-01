//! Every row so far, from both sources, counted once.

use analysis::planner::Row;
use analysis::reference::{self, Piece};
use analysis::technique::Fault;
use anyhow::Result;
use chrono::{DateTime, Local, NaiveDateTime};
use sqlx::SqlitePool;

use crate::concept2::C2Result;
use crate::sessions::SessionRow;

/// A logbook result this close to an app session is the same row, uploaded.
pub const SAME_ROW_MINUTES: i64 = 15;

/// Unix microseconds to local wall time.
pub fn local(us: i64) -> Option<NaiveDateTime> {
    DateTime::from_timestamp_micros(us).map(|t| t.with_timezone(&Local).naive_local())
}

#[derive(Debug, Clone)]
pub enum Entry {
    Logbook(Box<C2Result>),
    App(Box<SessionRow>),
}

impl Entry {
    /// Local start time.
    pub fn start(&self) -> NaiveDateTime {
        match self {
            Self::Logbook(r) => r.started_at,
            Self::App(s) => local(s.summary.started_at_us).unwrap_or_default(),
        }
    }

    pub fn day(&self) -> i64 {
        crate::health::day_number(self.start().date())
    }
}

/// Real rows, oldest first: app sessions (not simulated) and RowErg logbook
/// results, without logbook uploads of rows the app recorded. `except` (a
/// first catch, µs) leaves out that session and its upload.
pub async fn entries(pool: &SqlitePool, except: Option<i64>) -> Result<Vec<Entry>> {
    let sessions: Vec<SessionRow> = crate::sessions::list_sessions(pool)
        .await?
        .into_iter()
        .filter(|s| !s.meta.simulated)
        .collect();
    let app_starts: Vec<NaiveDateTime> = sessions
        .iter()
        .map(|s| s.summary.started_at_us)
        .chain(except)
        .filter_map(local)
        .collect();
    let mut out: Vec<Entry> = crate::db::list_concept2(pool)
        .await?
        .into_iter()
        .filter(|r| r.machine.eq_ignore_ascii_case("rowerg"))
        .filter(|r| !app_starts.iter().any(|t| (r.started_at - *t).num_minutes().abs() <= SAME_ROW_MINUTES))
        .map(|r| Entry::Logbook(Box::new(r)))
        .collect();
    out.extend(
        sessions
            .into_iter()
            .filter(|s| Some(s.summary.started_at_us) != except)
            .map(|s| Entry::App(Box::new(s))),
    );
    out.sort_by_key(Entry::start);
    Ok(out)
}

/// Steady pieces to compare a reference row with: every row but the one at
/// `except` (its first catch), logbook intervals left out.
pub async fn pieces(pool: &SqlitePool, except: i64) -> Result<Vec<Piece>> {
    let mut out = Vec::new();
    for e in entries(pool, Some(except)).await? {
        let label = format!("{} {}", e.start().date(), if matches!(e, Entry::App(_)) { "app" } else { "logbook" });
        let piece = match e {
            Entry::Logbook(r) if r.rest_time_s.is_none_or(|s| s <= 0.0) => {
                r.avg_watts.zip(r.avg_hr).zip(r.stroke_rate).map(|((watts, hr), spm)| Piece {
                    label,
                    minutes: r.work_time_s / 60.0,
                    watts,
                    spm,
                    hr: hr as f64,
                })
            }
            Entry::Logbook(_) => None,
            Entry::App(s) => {
                let w = &s.summary.whole;
                w.watts.zip(w.hr).zip(w.spm).map(|((watts, hr), spm)| Piece {
                    label,
                    minutes: w.seconds / 60.0,
                    watts,
                    spm,
                    hr,
                })
            }
        };
        out.extend(piece);
    }
    Ok(out)
}

/// Every row for the planner, reference rows with their verdict.
pub async fn planner_rows(pool: &SqlitePool) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    for e in entries(pool, None).await? {
        let day = e.day();
        rows.push(match e {
            Entry::Logbook(r) => Row {
                day,
                minutes: r.work_time_s / 60.0,
                plan: None,
                ramp: None,
            },
            Entry::App(s) => {
                let ramp = if s.meta.plan.as_deref() == Some("reference") {
                    let history = pieces(pool, s.summary.started_at_us).await?;
                    match reference::compare(&s.summary.whole, &history) {
                        reference::Outcome::Compared(c) => Some(c.ramp),
                        _ => None,
                    }
                } else {
                    None
                };
                Row {
                    day,
                    minutes: s.summary.whole.seconds / 60.0,
                    plan: s.meta.plan.clone(),
                    ramp,
                }
            }
        });
    }
    Ok(rows)
}

/// Faults rarer than this in recent sessions aren't worth a session's focus.
pub const FOCUS_MIN_RATE: f64 = 0.10;
pub const FOCUS_SESSIONS: usize = 3;

/// The fault with the highest rate (≥ 10%) over the last 3 real sessions;
/// camera faults only if this session has the camera.
pub async fn auto_focus(pool: &SqlitePool, camera: bool) -> Result<Option<(Fault, f64)>> {
    let sessions = crate::sessions::list_sessions(pool).await?;
    let recent: Vec<_> = sessions.iter().rev().filter(|s| !s.meta.simulated).take(FOCUS_SESSIONS).collect();
    let rate = |f: Fault| {
        let rates: Vec<f64> = recent
            .iter()
            .filter_map(|s| s.summary.faults.iter().find(|r| r.fault == f && r.of > 0))
            .map(|r| r.hits as f64 / r.of as f64)
            .collect();
        (!rates.is_empty()).then(|| rates.iter().sum::<f64>() / rates.len() as f64)
    };
    Ok(Fault::ALL
        .into_iter()
        .filter(|f| camera || !f.needs_camera())
        .filter_map(|f| rate(f).map(|r| (f, r)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|&(_, r)| r >= FOCUS_MIN_RATE))
}
