//! What the commands share: the database, the rower's profile, past pieces.

use std::path::PathBuf;

use analysis::reference::Piece;
use analysis::zones::{HeartProfile, Source};
use anyhow::Result;
use chrono::{DateTime, Local, NaiveDateTime};
use store::SqlitePool;
use store::sessions::Profile;

/// A logbook result this close to an app session is the same row, uploaded.
const SAME_ROW_MINUTES: i64 = 15;

pub struct Ctx {
    pub db_path: PathBuf,
    pub pool: SqlitePool,
    pub profile: Profile,
    pub age: Option<i64>,
}

impl Ctx {
    pub async fn open(db: Option<PathBuf>) -> Result<Self> {
        let db_path = db.unwrap_or_else(store::paths::db_path);
        let pool = store::db::open(&db_path).await?;
        let profile = store::sessions::get_profile(&pool).await?;
        let age = store::sessions::latest_age(&pool).await?;
        Ok(Self {
            db_path,
            pool,
            profile,
            age,
        })
    }

    /// Heart profile with estimates filling what isn't set.
    pub fn heart(&self) -> Option<(HeartProfile, Source, Source)> {
        HeartProfile::resolve(
            self.profile.hr_rest.map(f64::from),
            self.profile.hr_max.map(f64::from),
            self.age.map(|a| a as f64),
            self.profile.sex,
        )
    }

    /// "HRrest 60 (assumed), HRmax 184 (from age 35)".
    pub fn heart_note(&self) -> String {
        let Some((h, rest, max)) = self.heart() else {
            return "heart profile invalid: check `rowctl profile`".into();
        };
        let rest_note = match rest {
            Source::Set => String::new(),
            Source::Estimated => " (assumed)".into(),
        };
        let max_note = match (max, self.age) {
            (Source::Set, _) => String::new(),
            (Source::Estimated, Some(age)) => format!(" (estimated from age {age})"),
            (Source::Estimated, None) => " (assumed)".into(),
        };
        format!("HRrest {:.0}{rest_note}, HRmax {:.0}{max_note}", h.rest, h.max)
    }

    /// Steady pieces to compare a reference row with: logbook results and
    /// earlier app sessions, minus the session itself (`exclude_us`, its
    /// first catch) and logbook uploads of rows the app recorded.
    pub async fn history(&self, exclude_us: i64) -> Result<Vec<Piece>> {
        let sessions = store::sessions::list_sessions(&self.pool).await?;
        let app_starts: Vec<NaiveDateTime> = sessions
            .iter()
            .filter(|s| !s.meta.simulated)
            .map(|s| s.summary.started_at_us)
            .chain([exclude_us])
            .filter_map(local)
            .collect();

        let mut pieces = Vec::new();
        for r in store::db::list_concept2(&self.pool).await? {
            let interval = r.rest_time_s.is_some_and(|s| s > 0.0);
            let same_row = app_starts
                .iter()
                .any(|t| (r.started_at - *t).num_minutes().abs() <= SAME_ROW_MINUTES);
            if !r.machine.eq_ignore_ascii_case("rowerg") || interval || same_row {
                continue;
            }
            if let (Some(watts), Some(hr), Some(spm)) = (r.avg_watts, r.avg_hr, r.stroke_rate) {
                pieces.push(Piece {
                    label: format!("{} logbook", r.started_at.date()),
                    minutes: r.work_time_s / 60.0,
                    watts,
                    spm,
                    hr: hr as f64,
                });
            }
        }
        for s in sessions {
            let w = &s.summary.whole;
            if s.meta.simulated || s.summary.started_at_us == exclude_us {
                continue;
            }
            if let (Some(watts), Some(hr), Some(spm)) = (w.watts, w.hr, w.spm) {
                pieces.push(Piece {
                    label: format!("{} app", local(s.summary.started_at_us).map(|t| t.date().to_string()).unwrap_or_default()),
                    minutes: w.seconds / 60.0,
                    watts,
                    spm,
                    hr,
                });
            }
        }
        Ok(pieces)
    }
}

/// Unix microseconds to local wall time.
pub fn local(us: i64) -> Option<NaiveDateTime> {
    DateTime::from_timestamp_micros(us).map(|t| t.with_timezone(&Local).naive_local())
}
