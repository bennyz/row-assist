//! Every row so far, from both sources, counted once.

use analysis::planner::Row;
use analysis::reference::{self, Piece};
use analysis::technique::Fault;
use analysis::workout::{Kind, Progression, easy_power, four_by_four_hr_top, progression as next_power};
use analysis::zones::HeartProfile;
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

/// The heart profile the CLI and the server both use: what's set, else
/// the Watch's resting heart rate and an estimate from age.
pub async fn heart_profile(pool: &SqlitePool, today: i64) -> Result<Option<HeartProfile>> {
    let profile = crate::sessions::get_profile(pool).await?;
    let age = crate::sessions::latest_age(pool).await?;
    let recent = crate::health::list_days(pool, today - 30).await?;
    let watch_rest = analysis::readiness::zone_rest(&recent, today);
    Ok(profile.heart(age, watch_rest).map(|h| h.0))
}

/// The next `kind` workout's power: an interval workout's from the last
/// real one with its work intervals; an easy or steady row's from the last
/// real one of either long enough to measure its heart-rate drift. `None`
/// for the other kinds, or with no row to go on.
pub async fn progression(pool: &SqlitePool, kind: Kind, heart: Option<&HeartProfile>) -> Result<Option<Progression>> {
    let sessions = crate::sessions::list_sessions(pool).await?;
    let mut real = sessions.iter().rev().filter(|s| !s.meta.simulated);
    Ok(match kind {
        Kind::Easy | Kind::Minimum | Kind::Steady => real
            .filter(|s| matches!(s.meta.plan.as_deref().and_then(Kind::from_name), Some(Kind::Easy | Kind::Steady)))
            .find_map(|s| easy_power(&s.summary)),
        _ => real
            .find(|s| s.meta.plan.as_deref() == Some(kind.name()) && !s.summary.work.is_empty())
            .and_then(|s| next_power(kind, &s.summary.work, heart.map(four_by_four_hr_top), s.feel.effort)),
    })
}

/// Real sessions the focus is chosen from.
pub const FOCUS_SESSIONS: usize = 3;

/// The focus over the last 3 real sessions (`planner::focus`); camera
/// faults only if this session has the camera.
pub async fn auto_focus(pool: &SqlitePool, camera: bool) -> Result<Option<(Fault, f64)>> {
    let sessions = crate::sessions::list_sessions(pool).await?;
    let recent: Vec<_> = sessions
        .iter()
        .rev()
        .filter(|s| !s.meta.simulated)
        .take(FOCUS_SESSIONS)
        .map(|s| &s.summary)
        .collect();
    Ok(analysis::planner::focus(&recent, camera))
}
