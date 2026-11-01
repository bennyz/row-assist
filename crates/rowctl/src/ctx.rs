//! What the commands share: the database, the rower's profile, past pieces.

use std::path::PathBuf;

use analysis::planner::Row;
use analysis::reference::Piece;
use analysis::technique::Fault;
use analysis::zones::{HeartProfile, Source};
use anyhow::Result;
use store::SqlitePool;
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

    /// Steady pieces to compare a reference row with (all but `exclude_us`).
    pub async fn history(&self, exclude_us: i64) -> Result<Vec<Piece>> {
        store::history::pieces(&self.pool, exclude_us).await
    }
}

pub use store::history::local;

impl Ctx {
    pub async fn auto_focus(&self, camera: bool) -> Option<(Fault, f64)> {
        store::history::auto_focus(&self.pool, camera).await.ok().flatten()
    }

    /// Every row so far for the planner.
    pub async fn rows(&self) -> Result<Vec<Row>> {
        store::history::planner_rows(&self.pool).await
    }
}
