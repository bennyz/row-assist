//! What the commands share: the database, the rower's profile, past pieces.

use std::path::PathBuf;

use analysis::planner::Row;
use analysis::reference::{self, Piece};
use analysis::technique::Fault;
use analysis::zones::{HeartProfile, Source};
use anyhow::Result;
use store::SqlitePool;
use store::history::{Entry, entries};
use store::sessions::Profile;


pub struct Ctx {
    pub db_path: PathBuf,
    pub pool: SqlitePool,
    pub profile: Profile,
    pub age: Option<i64>,
    /// Two-week median resting HR from the Watch, when there's a week of it.
    pub watch_rest: Option<f64>,
}

/// Today as a day number (local).
pub fn today() -> i64 {
    store::health::day_number(chrono::Local::now().date_naive())
}

impl Ctx {
    pub async fn open(db: Option<PathBuf>) -> Result<Self> {
        let db_path = db.unwrap_or_else(store::paths::db_path);
        let pool = store::db::open(&db_path).await?;
        let profile = store::sessions::get_profile(&pool).await?;
        let age = store::sessions::latest_age(&pool).await?;
        let recent = store::health::list_days(&pool, today() - 30).await?;
        let watch_rest = analysis::readiness::zone_rest(&recent, today());
        Ok(Self {
            db_path,
            pool,
            profile,
            age,
            watch_rest,
        })
    }

    /// Heart profile with estimates filling what isn't set.
    pub fn heart(&self) -> Option<(HeartProfile, Source, Source)> {
        self.profile.heart(self.age, self.watch_rest)
    }

    /// "HRrest 60 (assumed), HRmax 184 (from age 35)".
    pub fn heart_note(&self) -> String {
        let Some((h, rest, max)) = self.heart() else {
            return "heart profile invalid: check `rowctl profile`".into();
        };
        let rest_note = match rest {
            Source::Set => String::new(),
            Source::Watch => " (Watch, 2-week median)".into(),
            Source::Estimated => " (assumed)".into(),
        };
        let max_note = match (max, self.age) {
            (Source::Set | Source::Watch, _) => String::new(),
            (Source::Estimated, Some(age)) => format!(" (estimated from age {age})"),
            (Source::Estimated, None) => " (assumed)".into(),
        };
        format!("HRrest {:.0}{rest_note}, HRmax {:.0}{max_note}", h.rest, h.max)
    }

    /// Steady pieces to compare a reference row with: logbook results and
    /// earlier app sessions, minus the session itself (`exclude_us`, its
    /// first catch) and logbook uploads of rows the app recorded.
    pub async fn history(&self, exclude_us: i64) -> Result<Vec<Piece>> {
        let mut pieces = Vec::new();
        for e in entries(&self.pool, Some(exclude_us)).await? {
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
            pieces.extend(piece);
        }
        Ok(pieces)
    }
}

pub use store::history::local;

impl Ctx {
    /// The fault with the highest rate (≥ 10%) over the last 3 real sessions;
    /// camera faults only if this session has the camera.
    pub async fn auto_focus(&self, camera: bool) -> Option<(Fault, f64)> {
        let sessions = store::sessions::list_sessions(&self.pool).await.ok()?;
        let recent: Vec<_> = sessions.iter().rev().filter(|s| !s.meta.simulated).take(FOCUS_SESSIONS).collect();
        let rate = |f: Fault| {
            let rates: Vec<f64> = recent
                .iter()
                .filter_map(|s| s.summary.faults.iter().find(|r| r.fault == f && r.of > 0))
                .map(|r| r.hits as f64 / r.of as f64)
                .collect();
            (!rates.is_empty()).then(|| rates.iter().sum::<f64>() / rates.len() as f64)
        };
        Fault::ALL
            .into_iter()
            .filter(|f| camera || !f.needs_camera())
            .filter_map(|f| rate(f).map(|r| (f, r)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .filter(|&(_, r)| r >= FOCUS_MIN_RATE)
    }

    /// Every row so far for the planner, reference rows with their verdict.
    pub async fn rows(&self) -> Result<Vec<Row>> {
        let mut rows = Vec::new();
        for e in entries(&self.pool, None).await? {
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
                        let history = self.history(s.summary.started_at_us).await?;
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
}

/// Faults rarer than this in recent sessions aren't worth a session's focus.
const FOCUS_MIN_RATE: f64 = 0.10;
const FOCUS_SESSIONS: usize = 3;
