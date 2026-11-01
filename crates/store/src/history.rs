//! Every row so far, from both sources, counted once.

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
