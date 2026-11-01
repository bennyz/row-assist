//! SQLite database. Schema lives in the repo-root `migrations/`, embedded at
//! compile time and applied on open.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow};
use sqlx::{Row, SqlitePool};

use crate::concept2::{C2Result, DATE_FORMAT};

pub async fn open(path: &Path) -> Result<SqlitePool> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await
        .with_context(|| format!("open {}", path.display()))?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    Ok(pool)
}

/// Insert or update by `log_id`. Returns how many rows were new.
pub async fn upsert_concept2(pool: &SqlitePool, rows: &[C2Result]) -> Result<u64> {
    let before = count_concept2(pool).await?;
    let mut tx = pool.begin().await?;
    for r in rows {
        sqlx::query(
            "INSERT INTO concept2_results (
                log_id, started_at, description, work_time_s, rest_time_s,
                work_distance_m, rest_distance_m, stroke_rate, stroke_count,
                pace_s_per_500, avg_watts, cal_per_hour, total_cal, avg_hr,
                drag_factor, age, weight_class, machine, ranked, comments, entered_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT (log_id) DO UPDATE SET
                started_at = excluded.started_at,
                description = excluded.description,
                work_time_s = excluded.work_time_s,
                rest_time_s = excluded.rest_time_s,
                work_distance_m = excluded.work_distance_m,
                rest_distance_m = excluded.rest_distance_m,
                stroke_rate = excluded.stroke_rate,
                stroke_count = excluded.stroke_count,
                pace_s_per_500 = excluded.pace_s_per_500,
                avg_watts = excluded.avg_watts,
                cal_per_hour = excluded.cal_per_hour,
                total_cal = excluded.total_cal,
                avg_hr = excluded.avg_hr,
                drag_factor = excluded.drag_factor,
                age = excluded.age,
                weight_class = excluded.weight_class,
                machine = excluded.machine,
                ranked = excluded.ranked,
                comments = excluded.comments,
                entered_at = excluded.entered_at",
        )
        .bind(r.log_id)
        .bind(r.started_at.format(DATE_FORMAT).to_string())
        .bind(&r.description)
        .bind(r.work_time_s)
        .bind(r.rest_time_s)
        .bind(r.work_distance_m)
        .bind(r.rest_distance_m)
        .bind(r.stroke_rate)
        .bind(r.stroke_count)
        .bind(r.pace_s_per_500)
        .bind(r.avg_watts)
        .bind(r.cal_per_hour)
        .bind(r.total_cal)
        .bind(r.avg_hr)
        .bind(r.drag_factor)
        .bind(r.age)
        .bind(&r.weight_class)
        .bind(&r.machine)
        .bind(r.ranked)
        .bind(&r.comments)
        .bind(r.entered_at.map(|t| t.format(DATE_FORMAT).to_string()))
        .execute(&mut *tx)
        .await
        .with_context(|| format!("upsert log {}", r.log_id))?;
    }
    tx.commit().await?;
    Ok(count_concept2(pool).await? - before)
}

pub async fn count_concept2(pool: &SqlitePool) -> Result<u64> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM concept2_results")
        .fetch_one(pool)
        .await?;
    Ok(n as u64)
}

/// All results, oldest first.
pub async fn list_concept2(pool: &SqlitePool) -> Result<Vec<C2Result>> {
    let rows = sqlx::query("SELECT * FROM concept2_results ORDER BY started_at, log_id")
        .fetch_all(pool)
        .await?;
    rows.iter().map(from_row).collect()
}

fn from_row(row: &SqliteRow) -> Result<C2Result> {
    let date = |col: &str| -> Result<Option<NaiveDateTime>> {
        let s: Option<String> = row.try_get(col)?;
        s.map(|s| NaiveDateTime::parse_from_str(&s, DATE_FORMAT).with_context(|| format!("bad {col} {s:?}")))
            .transpose()
    };
    Ok(C2Result {
        log_id: row.try_get("log_id")?,
        started_at: date("started_at")?.context("started_at is NULL")?,
        description: row.try_get("description")?,
        work_time_s: row.try_get("work_time_s")?,
        rest_time_s: row.try_get("rest_time_s")?,
        work_distance_m: row.try_get("work_distance_m")?,
        rest_distance_m: row.try_get("rest_distance_m")?,
        stroke_rate: row.try_get("stroke_rate")?,
        stroke_count: row.try_get("stroke_count")?,
        pace_s_per_500: row.try_get("pace_s_per_500")?,
        avg_watts: row.try_get("avg_watts")?,
        cal_per_hour: row.try_get("cal_per_hour")?,
        total_cal: row.try_get("total_cal")?,
        avg_hr: row.try_get("avg_hr")?,
        drag_factor: row.try_get("drag_factor")?,
        age: row.try_get("age")?,
        weight_class: row.try_get("weight_class")?,
        machine: row.try_get("machine")?,
        ranked: row.try_get("ranked")?,
        comments: row.try_get("comments")?,
        entered_at: date("entered_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::concept2::parse_csv;

    const SAMPLE: &str = r#""Log ID",Date,Description,"Work Time (Formatted)","Work Time (Seconds)","Rest Time (Formatted)","Rest Time (Seconds)","Work Distance","Rest Distance","Stroke Rate/Cadence","Stroke Count",Pace,"Avg Watts",Cal/Hour,"Total Cal","Avg Heart Rate","Drag Factor",Age,Weight,Type,Ranked,Comments,"Date Entered"
2002,"2026-01-03 07:30:00","10:00 row",10:00.0,600,,,2000,,20,200,2:30.0,104,700,115,0,110,30,Hwt,RowErg,No,,"2026-01-03 05:31:00"
2001,"2026-01-02 07:30:00","20:00 row",20:00.0,1200,,,4000,,20,400,2:30.0,104,700,230,140,110,30,Hwt,RowErg,No,"note",
"#;

    #[tokio::test]
    async fn import_is_idempotent_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("row-assist-test-{}", std::process::id()));
        let path = dir.join("test.db");
        let _ = std::fs::remove_dir_all(&dir);
        let pool = open(&path).await.unwrap();

        let rows = parse_csv(SAMPLE.as_bytes()).unwrap();
        assert_eq!(upsert_concept2(&pool, &rows).await.unwrap(), 2);
        assert_eq!(upsert_concept2(&pool, &rows).await.unwrap(), 0, "re-import adds nothing");

        let back = list_concept2(&pool).await.unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].log_id, 2001, "oldest first");
        let mut expected = rows.clone();
        expected.sort_by_key(|r| r.started_at);
        assert_eq!(back, expected);

        pool.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
