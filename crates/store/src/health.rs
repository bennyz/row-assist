//! Apple Watch recovery data, a row per day, and where it comes from: the
//! rower's own record on the home server (nutridag), which the phone posts
//! to every day and which, unlike this computer, is always on to receive it.

use std::path::Path;

use analysis::readiness::HealthDay;
use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use serde::Deserialize;
use sqlx::{Row, SqlitePool};

fn epoch() -> NaiveDate {
    NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
}

pub fn day_number(d: NaiveDate) -> i64 {
    (d - epoch()).num_days()
}

pub fn day_date(n: i64) -> NaiveDate {
    epoch() + chrono::Duration::days(n)
}

/// What the home server answers at `<link>/days`: its days, each with
/// whichever metrics it has (steps, weight and the rest are not ours).
#[derive(Debug, Deserialize)]
struct Answer {
    days: Vec<AnswerDay>,
}

#[derive(Debug, Deserialize)]
struct AnswerDay {
    day: String,
    /// The day's mean SDNN, ms.
    hrv: Option<f64>,
    resting_hr: Option<f64>,
    /// Asleep, for the morning the night ended.
    sleep_hours: Option<f64>,
    vo2max: Option<f64>,
    strength_minutes: Option<f64>,
    steps: Option<f64>,
    exercise_minutes: Option<f64>,
}

/// The days in the home server's answer that carry anything of ours.
pub fn parse_days(answer: &[u8]) -> Result<Vec<HealthDay>> {
    let answer: Answer = serde_json::from_slice(answer).context("not the list of days expected")?;
    answer
        .days
        .iter()
        .filter(|d| {
            [d.hrv, d.resting_hr, d.sleep_hours, d.vo2max, d.strength_minutes, d.steps, d.exercise_minutes]
                .iter()
                .any(Option::is_some)
        })
        .map(|d| {
            let day = NaiveDate::parse_from_str(&d.day, "%Y-%m-%d").with_context(|| format!("bad day {:?}", d.day))?;
            Ok(HealthDay {
                day: day_number(day),
                hrv_ms: d.hrv,
                resting_hr: d.resting_hr,
                sleep_h: d.sleep_hours,
                vo2max: d.vo2max,
                strength_min: d.strength_minutes,
                steps: d.steps,
                exercise_min: d.exercise_minutes,
            })
        })
        .collect()
}

/// Insert or update by day; a value that's missing keeps what was stored.
pub async fn upsert_days(pool: &SqlitePool, days: &[HealthDay]) -> Result<usize> {
    let mut tx = pool.begin().await?;
    for d in days {
        sqlx::query(
            "INSERT INTO health_daily (day, hrv_ms, resting_hr, sleep_h, vo2max, strength_min, steps, exercise_min)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (day) DO UPDATE SET
                hrv_ms = COALESCE(excluded.hrv_ms, hrv_ms),
                resting_hr = COALESCE(excluded.resting_hr, resting_hr),
                sleep_h = COALESCE(excluded.sleep_h, sleep_h),
                vo2max = COALESCE(excluded.vo2max, vo2max),
                strength_min = COALESCE(excluded.strength_min, strength_min),
                steps = COALESCE(excluded.steps, steps),
                exercise_min = COALESCE(excluded.exercise_min, exercise_min),
                updated_at = datetime('now')",
        )
        .bind(day_date(d.day).to_string())
        .bind(d.hrv_ms)
        .bind(d.resting_hr)
        .bind(d.sleep_h)
        .bind(d.vo2max)
        .bind(d.strength_min)
        .bind(d.steps)
        .bind(d.exercise_min)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(days.len())
}

/// Days from `from` (a day number) on, oldest first.
pub async fn list_days(pool: &SqlitePool, from: i64) -> Result<Vec<HealthDay>> {
    let rows = sqlx::query(
        "SELECT day, hrv_ms, resting_hr, sleep_h, vo2max, strength_min, steps, exercise_min
         FROM health_daily WHERE day >= ? ORDER BY day",
    )
        .bind(day_date(from).to_string())
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|r| {
            let day: String = r.try_get("day")?;
            Ok(HealthDay {
                day: day_number(NaiveDate::parse_from_str(&day, "%Y-%m-%d").with_context(|| format!("bad day {day:?}"))?),
                hrv_ms: r.try_get("hrv_ms")?,
                resting_hr: r.try_get("resting_hr")?,
                sleep_h: r.try_get("sleep_h")?,
                vo2max: r.try_get("vo2max")?,
                strength_min: r.try_get("strength_min")?,
                steps: r.try_get("steps")?,
                exercise_min: r.try_get("exercise_min")?,
            })
        })
        .collect()
}

/// The rower's health link on the home server, as nutridag's `/health` gives
/// it (`http://<server>/health/<secret>`), if one has been set. It is a
/// credential: kept beside the database, readable by the owner only, and
/// never printed or logged.
pub fn source(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(link) => Ok(Some(link.trim().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

pub fn set_source(path: &Path, link: &str) -> Result<()> {
    let link = link.trim().trim_end_matches('/');
    // The home server speaks plain HTTP on the home network, and so do we.
    if !link.starts_with("http://") || !link.contains("/health/") {
        bail!("expected the link nutridag's /health command gives: http://<server>/health/<secret>");
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, link).with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open;

    #[test]
    fn the_home_servers_days_become_ours() {
        let answer = br#"{"days": [
            {"day": "2026-10-31", "steps": 8532.0, "weight": 70.0},
            {"day": "2026-11-01", "hrv": 55.0, "resting_hr": 56.0, "sleep_hours": 7.0, "steps": 9000.0,
             "exercise_minutes": 42.0, "strength_minutes": 35.0, "vo2max": 38.4, "active_energy": 610.0},
            {"day": "2026-11-02", "sleep_hours": 6.5},
            {"day": "2026-11-03", "weight": 70.2}
        ]}"#;
        let nov1 = day_number(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
        assert_eq!(
            parse_days(answer).unwrap(),
            vec![
                HealthDay { day: nov1 - 1, steps: Some(8532.0), ..Default::default() },
                HealthDay {
                    day: nov1,
                    hrv_ms: Some(55.0),
                    resting_hr: Some(56.0),
                    sleep_h: Some(7.0),
                    vo2max: Some(38.4),
                    strength_min: Some(35.0),
                    steps: Some(9000.0),
                    exercise_min: Some(42.0),
                },
                HealthDay { day: nov1 + 1, sleep_h: Some(6.5), ..Default::default() },
            ],
            "a day of weight alone is not ours"
        );
        assert!(parse_days(br#"{"days": [{"day": "yesterday", "hrv": 1}]}"#).is_err());
        assert!(parse_days(b"<html>").is_err());
    }

    #[tokio::test]
    async fn upserts_keep_what_a_later_fetch_leaves_out() {
        let dir = std::env::temp_dir().join(format!("row-assist-health-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pool = open(&dir.join("t.db")).await.unwrap();
        let day = 20_400;
        let first = HealthDay { day, hrv_ms: Some(50.0), resting_hr: Some(56.0), sleep_h: Some(7.0), ..Default::default() };
        upsert_days(&pool, &[first]).await.unwrap();
        let later = HealthDay { day, hrv_ms: Some(52.0), resting_hr: None, sleep_h: None, ..Default::default() };
        upsert_days(&pool, &[later]).await.unwrap();
        let back = list_days(&pool, day - 1).await.unwrap();
        assert_eq!(back, vec![HealthDay { day, hrv_ms: Some(52.0), resting_hr: Some(56.0), sleep_h: Some(7.0), ..Default::default() }]);
        // The day's movement arrives later in the day and joins the same row.
        let moved = HealthDay { day, strength_min: Some(30.0), steps: Some(7200.0), exercise_min: Some(38.0), vo2max: Some(39.1), ..Default::default() };
        upsert_days(&pool, &[moved]).await.unwrap();
        let back = list_days(&pool, day - 1).await.unwrap();
        assert_eq!((back[0].hrv_ms, back[0].strength_min, back[0].vo2max), (Some(52.0), Some(30.0), Some(39.1)));

        let path = dir.join("health-source");
        assert_eq!(source(&path).unwrap(), None);
        assert!(set_source(&path, "https://elsewhere.example/").is_err());
        set_source(&path, " http://home:8080/health/s3cret-s3cret-s3cret/\n").unwrap();
        assert_eq!(source(&path).unwrap().as_deref(), Some("http://home:8080/health/s3cret-s3cret-s3cret"));
        pool.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
