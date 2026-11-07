//! Apple Watch recovery data: the samples an iPhone Shortcut posts, turned
//! into one row per day, and the token that guards the endpoint.

use std::collections::BTreeMap;
use std::path::Path;

use analysis::readiness::HealthDay;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, FixedOffset, NaiveDate};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sqlx::{Row, SqlitePool};

/// What the Shortcut sends. Every list is optional.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Samples {
    #[serde(deserialize_with = "lenient_list")]
    pub hrv: Vec<Sample>,
    #[serde(deserialize_with = "lenient_list")]
    pub resting_hr: Vec<Sample>,
    #[serde(deserialize_with = "lenient_list")]
    pub sleep: Vec<SleepSample>,
}

/// A list, or what Shortcuts makes of a list of dictionaries put into a
/// dictionary: one text with each item's JSON on a line of its own (or a
/// JSON array as text). A single item may come without the list.
fn lenient_list<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: DeserializeOwned,
{
    use serde::de::Error;
    use serde_json::Value;
    let items = match Value::deserialize(d)? {
        Value::Array(items) => items,
        Value::Null => vec![],
        item @ Value::Object(_) => vec![item],
        Value::String(text) if text.trim_start().starts_with('[') => {
            serde_json::from_str(&text).map_err(|e| D::Error::custom(format!("a list as text that isn't JSON: {e}")))?
        }
        Value::String(text) => text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| {
                serde_json::from_str(line).map_err(|e| D::Error::custom(format!("a line that isn't a sample: {e}")))
            })
            .collect::<Result<_, _>>()?,
        _ => return Err(D::Error::custom("expected a list of samples")),
    };
    items
        .into_iter()
        .map(|item| serde_json::from_value(item).map_err(D::Error::custom))
        .collect()
}

#[derive(Debug, Deserialize)]
pub struct Sample {
    pub date: String,
    #[serde(deserialize_with = "lenient_number")]
    pub value: f64,
}

/// A number, or text starting with one ("48 ms"): Shortcuts can send either,
/// depending on how the dictionary was built.
fn lenient_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    use serde::de::Error;
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::Number(n) => n.as_f64().ok_or_else(|| D::Error::custom("number out of range")),
        serde_json::Value::String(s) => s
            .trim()
            .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .next()
            .and_then(|t| t.parse().ok())
            .ok_or_else(|| D::Error::custom(format!("not a number: {s:?}"))),
        other => Err(D::Error::custom(format!("not a number: {other}"))),
    }
}

#[derive(Debug, Deserialize)]
pub struct SleepSample {
    pub start: String,
    pub end: String,
    /// "Core", "Deep", "REM", "Asleep", "In Bed", "Awake"…
    #[serde(default)]
    pub value: String,
}

/// A timestamp as Shortcuts or a Health export writes it: ISO 8601,
/// `2026-11-01 03:12:00 +0200`, or a bare date. Keeps its own offset, so the
/// day is the local day it was recorded.
fn parse_time(s: &str) -> Result<DateTime<FixedOffset>> {
    let s = s.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Ok(t);
    }
    if let Ok(t) = DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S %z") {
        return Ok(t);
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(12, 0, 0).unwrap().and_utc().fixed_offset());
    }
    bail!("unrecognised date {s:?}")
}

fn epoch() -> NaiveDate {
    NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
}

pub fn day_number(d: NaiveDate) -> i64 {
    (d - epoch()).num_days()
}

pub fn day_date(n: i64) -> NaiveDate {
    epoch() + chrono::Duration::days(n)
}

fn is_asleep(value: &str) -> bool {
    let v = value.to_ascii_lowercase().replace(' ', "");
    !(v.contains("inbed") || v.contains("awake"))
}

/// One row per day: mean HRV, mean resting HR, and sleep as the union of
/// asleep intervals (phone and Watch both log sleep; summing would double it),
/// counted for the day the night ended.
pub fn aggregate(s: &Samples) -> Result<Vec<HealthDay>> {
    #[derive(Default)]
    struct Acc {
        hrv: Vec<f64>,
        rhr: Vec<f64>,
        sleep: Vec<(i64, i64)>,
    }
    let mut days: BTreeMap<i64, Acc> = BTreeMap::new();
    for x in &s.hrv {
        let t = parse_time(&x.date)?;
        if x.value > 0.0 {
            days.entry(day_number(t.date_naive())).or_default().hrv.push(x.value);
        }
    }
    for x in &s.resting_hr {
        let t = parse_time(&x.date)?;
        if x.value > 0.0 {
            days.entry(day_number(t.date_naive())).or_default().rhr.push(x.value);
        }
    }
    for x in s.sleep.iter().filter(|x| is_asleep(&x.value)) {
        let (a, b) = (parse_time(&x.start)?, parse_time(&x.end)?);
        if b > a {
            days.entry(day_number(b.date_naive()))
                .or_default()
                .sleep
                .push((a.timestamp(), b.timestamp()));
        }
    }
    let mean = |v: &[f64]| (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64);
    Ok(days
        .into_iter()
        .map(|(day, mut acc)| HealthDay {
            day,
            hrv_ms: mean(&acc.hrv),
            resting_hr: mean(&acc.rhr),
            sleep_h: (!acc.sleep.is_empty()).then(|| union_seconds(&mut acc.sleep) as f64 / 3600.0),
        })
        .collect())
}

fn union_seconds(spans: &mut [(i64, i64)]) -> i64 {
    spans.sort_unstable();
    let mut total = 0;
    let mut cur: Option<(i64, i64)> = None;
    for &(a, b) in spans.iter() {
        cur = match cur {
            Some((ca, cb)) if a <= cb => Some((ca, cb.max(b))),
            Some((ca, cb)) => {
                total += cb - ca;
                Some((a, b))
            }
            None => Some((a, b)),
        };
    }
    total + cur.map_or(0, |(a, b)| b - a)
}

/// Insert or update by day; a value that's missing keeps what was stored.
pub async fn upsert_days(pool: &SqlitePool, days: &[HealthDay]) -> Result<usize> {
    let mut tx = pool.begin().await?;
    for d in days {
        sqlx::query(
            "INSERT INTO health_daily (day, hrv_ms, resting_hr, sleep_h) VALUES (?, ?, ?, ?)
             ON CONFLICT (day) DO UPDATE SET
                hrv_ms = COALESCE(excluded.hrv_ms, hrv_ms),
                resting_hr = COALESCE(excluded.resting_hr, resting_hr),
                sleep_h = COALESCE(excluded.sleep_h, sleep_h),
                updated_at = datetime('now')",
        )
        .bind(day_date(d.day).to_string())
        .bind(d.hrv_ms)
        .bind(d.resting_hr)
        .bind(d.sleep_h)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(days.len())
}

/// Days from `from` (a day number) on, oldest first.
pub async fn list_days(pool: &SqlitePool, from: i64) -> Result<Vec<HealthDay>> {
    let rows = sqlx::query("SELECT day, hrv_ms, resting_hr, sleep_h FROM health_daily WHERE day >= ? ORDER BY day")
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
            })
        })
        .collect()
}

/// The shared secret the Shortcut sends, created on first use (32 hex chars
/// from the OS's random source) and kept beside the database.
pub fn token(path: &Path) -> Result<String> {
    if let Ok(t) = std::fs::read_to_string(path) {
        let t = t.trim().to_string();
        if t.len() >= 16 {
            return Ok(t);
        }
    }
    let mut bytes = [0u8; 16];
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .context("read /dev/urandom")?;
    let t: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, &t).with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open;

    const POST: &str = r#"{
        "hrv": [
            {"date": "2026-11-01T03:12:00+02:00", "value": 50},
            {"date": "2026-11-01T14:00:00+02:00", "value": "60 ms"},
            {"date": "2026-10-31 23:30:00 +0200", "value": 40}
        ],
        "resting_hr": [{"date": "2026-11-01", "value": "56 count/min"}],
        "sleep": [
            {"start": "2026-10-31T23:30:00+02:00", "end": "2026-11-01T02:00:00+02:00", "value": "Core"},
            {"start": "2026-11-01T01:00:00+02:00", "end": "2026-11-01T06:30:00+02:00", "value": "Asleep"},
            {"start": "2026-10-31T23:00:00+02:00", "end": "2026-11-01T07:00:00+02:00", "value": "In Bed"},
            {"start": "2026-11-01T06:30:00+02:00", "end": "2026-11-01T06:40:00+02:00", "value": "Awake"}
        ]
    }"#;

    #[test]
    fn samples_become_days() {
        let s: Samples = serde_json::from_str(POST).unwrap();
        let days = aggregate(&s).unwrap();
        let nov1 = day_number(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
        assert_eq!(days.len(), 2);
        let d = days.iter().find(|d| d.day == nov1).unwrap();
        assert_eq!(d.hrv_ms, Some(55.0));
        assert_eq!(d.resting_hr, Some(56.0));
        // 23:30–06:30 asleep (two overlapping sources), in bed and awake excluded.
        assert_eq!(d.sleep_h, Some(7.0));
        let oct31 = days.iter().find(|d| d.day == nov1 - 1).unwrap();
        assert_eq!((oct31.hrv_ms, oct31.sleep_h), (Some(40.0), None));
        assert!(aggregate(&serde_json::from_str(r#"{"hrv": [{"date": "yesterday", "value": 1}]}"#).unwrap()).is_err());
    }

    /// A list variable of dictionaries, put into the request's dictionary,
    /// reaches us as text: one item's JSON per line, values as text.
    #[test]
    fn lists_sent_as_text_one_item_per_line_are_read() {
        let lines = |items: &[&str]| serde_json::Value::String(items.join("\n"));
        let post = serde_json::json!({
            "hrv": lines(&[
                r#"{"value":"50.0","date":"2026-11-01T03:12:00+02:00"}"#,
                r#"{"value":"60.0","date":"2026-11-01T14:00:00+02:00"}"#,
            ]),
            "resting_hr": r#"[{"value": "56", "date": "2026-11-01"}]"#,
            "sleep": {"start": "2026-10-31T23:30:00+02:00", "end": "2026-11-01T06:30:00+02:00", "value": "Core"},
        });
        let s: Samples = serde_json::from_value(post).unwrap();
        let days = aggregate(&s).unwrap();
        let nov1 = day_number(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
        let d = days.iter().find(|d| d.day == nov1).unwrap();
        assert_eq!((d.hrv_ms, d.resting_hr, d.sleep_h), (Some(55.0), Some(56.0), Some(7.0)));

        let bad = serde_json::json!({"hrv": "not a sample"});
        let e = serde_json::from_value::<Samples>(bad).unwrap_err().to_string();
        assert!(e.contains("a line that isn't a sample"), "{e}");
    }

    #[tokio::test]
    async fn upserts_keep_what_a_later_post_leaves_out() {
        let dir = std::env::temp_dir().join(format!("row-assist-health-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pool = open(&dir.join("t.db")).await.unwrap();
        let day = 20_400;
        let first = HealthDay { day, hrv_ms: Some(50.0), resting_hr: Some(56.0), sleep_h: Some(7.0) };
        upsert_days(&pool, &[first]).await.unwrap();
        let later = HealthDay { day, hrv_ms: Some(52.0), resting_hr: None, sleep_h: None };
        upsert_days(&pool, &[later]).await.unwrap();
        let back = list_days(&pool, day - 1).await.unwrap();
        assert_eq!(back, vec![HealthDay { day, hrv_ms: Some(52.0), resting_hr: Some(56.0), sleep_h: Some(7.0) }]);

        let path = dir.join("api-token");
        let t = token(&path).unwrap();
        assert_eq!(t.len(), 32);
        assert_eq!(token(&path).unwrap(), t, "stable once created");
        pool.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
