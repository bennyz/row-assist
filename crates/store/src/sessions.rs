//! App sessions, their strokes, and the rower's profile.

use analysis::session::Summary;
use analysis::strokes::StrokeRecord;
use analysis::zones::Sex;
use anyhow::{Context, Result, bail};
use sqlx::{Row, SqlitePool};

/// What identifies and describes a recording, from its header.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionMeta {
    pub started_at_us: i64,
    pub recording: String,
    pub note: Option<String>,
    pub plan: Option<String>,
    pub simulated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    pub id: i64,
    pub meta: SessionMeta,
    pub summary: Summary,
}

/// Insert or replace the session for this recording, with all its strokes.
pub async fn save_session(
    pool: &SqlitePool,
    meta: &SessionMeta,
    summary: &Summary,
    strokes: &[StrokeRecord],
) -> Result<i64> {
    let w = &summary.whole;
    let mut tx = pool.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO sessions (
            started_at_us, recording, note, plan, simulated, analysis_version,
            seconds, strokes, distance_m, watts, spm, hr, ef, decoupling_pct,
            watts_at_20, drag_factor, peak_hr, trimp, summary
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT (started_at_us) DO UPDATE SET
            recording = excluded.recording,
            note = excluded.note,
            plan = excluded.plan,
            simulated = excluded.simulated,
            analysis_version = excluded.analysis_version,
            seconds = excluded.seconds,
            strokes = excluded.strokes,
            distance_m = excluded.distance_m,
            watts = excluded.watts,
            spm = excluded.spm,
            hr = excluded.hr,
            ef = excluded.ef,
            decoupling_pct = excluded.decoupling_pct,
            watts_at_20 = excluded.watts_at_20,
            drag_factor = excluded.drag_factor,
            peak_hr = excluded.peak_hr,
            trimp = excluded.trimp,
            summary = excluded.summary,
            analyzed_at = datetime('now')
        RETURNING id",
    )
    .bind(meta.started_at_us)
    .bind(&meta.recording)
    .bind(&meta.note)
    .bind(&meta.plan)
    .bind(meta.simulated)
    .bind(summary.analysis_version)
    .bind(w.seconds)
    .bind(w.strokes)
    .bind(w.distance_m)
    .bind(w.watts)
    .bind(w.spm)
    .bind(w.hr)
    .bind(w.ef)
    .bind(summary.decoupling_pct)
    .bind(summary.rate_20.as_ref().and_then(|s| s.watts))
    .bind(summary.drag_factor)
    .bind(summary.heart.as_ref().and_then(|h| h.peak_10s_bpm))
    .bind(summary.heart.as_ref().and_then(|h| h.trimp))
    .bind(serde_json::to_string(summary)?)
    .fetch_one(&mut *tx)
    .await
    .context("save session")?;

    sqlx::query("DELETE FROM strokes WHERE session_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    for s in strokes {
        sqlx::query(
            "INSERT INTO strokes (
                session_id, idx, t_catch_us, drive_ms, recovery_ms, watts, spm,
                distance_m, drag_factor, hr, peak_n, record
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(s.index)
        .bind(s.t_catch_us)
        .bind(s.drive_ms)
        .bind(s.recovery_ms)
        .bind(s.watts)
        .bind(s.spm)
        .bind(s.distance_m)
        .bind(s.drag_factor)
        .bind(s.hr_bpm)
        .bind(s.peak_n)
        .bind(serde_json::to_string(s)?)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("save stroke {}", s.index))?;
    }
    tx.commit().await?;
    Ok(id)
}

/// All sessions, oldest first.
pub async fn list_sessions(pool: &SqlitePool) -> Result<Vec<SessionRow>> {
    let rows = sqlx::query(
        "SELECT id, started_at_us, recording, note, plan, simulated, summary
         FROM sessions ORDER BY started_at_us",
    )
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| {
            let summary: String = r.try_get("summary")?;
            Ok(SessionRow {
                id: r.try_get("id")?,
                meta: SessionMeta {
                    started_at_us: r.try_get("started_at_us")?,
                    recording: r.try_get("recording")?,
                    note: r.try_get("note")?,
                    plan: r.try_get("plan")?,
                    simulated: r.try_get("simulated")?,
                },
                summary: serde_json::from_str(&summary).context("stored summary")?,
            })
        })
        .collect()
}

pub async fn session_strokes(pool: &SqlitePool, session_id: i64) -> Result<Vec<StrokeRecord>> {
    let rows: Vec<String> = sqlx::query_scalar("SELECT record FROM strokes WHERE session_id = ? ORDER BY idx")
        .bind(session_id)
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|r| serde_json::from_str(r).context("stored stroke"))
        .collect()
}

/// The rower's settings; every field is optional until set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Profile {
    pub hr_rest: Option<u16>,
    pub hr_max: Option<u16>,
    pub sex: Option<Sex>,
    pub drag_target: Option<u8>,
}

fn sex_str(s: Sex) -> &'static str {
    match s {
        Sex::Male => "male",
        Sex::Female => "female",
    }
}

pub async fn get_profile(pool: &SqlitePool) -> Result<Profile> {
    let Some(r) = sqlx::query("SELECT hr_rest, hr_max, sex, drag_target FROM profile WHERE id = 1")
        .fetch_optional(pool)
        .await?
    else {
        return Ok(Profile::default());
    };
    let sex: Option<String> = r.try_get("sex")?;
    Ok(Profile {
        hr_rest: r.try_get("hr_rest")?,
        hr_max: r.try_get("hr_max")?,
        sex: match sex.as_deref() {
            None => None,
            Some("male") => Some(Sex::Male),
            Some("female") => Some(Sex::Female),
            Some(other) => bail!("bad sex in profile: {other:?}"),
        },
        drag_target: r.try_get("drag_target")?,
    })
}

pub async fn set_profile(pool: &SqlitePool, p: &Profile) -> Result<()> {
    sqlx::query(
        "INSERT INTO profile (id, hr_rest, hr_max, sex, drag_target) VALUES (1, ?, ?, ?, ?)
         ON CONFLICT (id) DO UPDATE SET
            hr_rest = excluded.hr_rest,
            hr_max = excluded.hr_max,
            sex = excluded.sex,
            drag_target = excluded.drag_target,
            updated_at = datetime('now')",
    )
    .bind(p.hr_rest)
    .bind(p.hr_max)
    .bind(p.sex.map(sex_str))
    .bind(p.drag_target)
    .execute(pool)
    .await?;
    Ok(())
}

/// The age on the most recent logbook result, for estimating HRmax.
pub async fn latest_age(pool: &SqlitePool) -> Result<Option<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT age FROM concept2_results WHERE age IS NOT NULL ORDER BY started_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open;
    use analysis::session::Segment;

    fn stroke(i: u32) -> StrokeRecord {
        StrokeRecord {
            index: i,
            t_catch_us: i as i64 * 3_000_000,
            t_finish_us: None,
            t_next_catch_us: None,
            drive_ms: Some(1000.0),
            recovery_ms: None,
            ratio: None,
            watts: Some(80),
            spm: Some(20),
            distance_m: Some(9.0),
            drag_factor: Some(110),
            hr_bpm: Some(140.0),
            peak_n: Some(500.0),
            force_n: vec![1.0, 2.0],
        }
    }

    #[tokio::test]
    async fn sessions_round_trip_and_reanalysis_replaces() {
        let dir = std::env::temp_dir().join(format!("row-assist-sessions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pool = open(&dir.join("t.db")).await.unwrap();

        let meta = SessionMeta {
            started_at_us: 1_700_000_000_000_000,
            recording: "/x/a.rowrec".into(),
            note: Some("reference row".into()),
            plan: Some("reference".into()),
            simulated: false,
        };
        let mut summary = Summary {
            analysis_version: 1,
            whole: Segment {
                strokes: 3,
                seconds: 9.0,
                watts: Some(80.0),
                ..Default::default()
            },
            ..Default::default()
        };
        let strokes: Vec<_> = (0..3).map(stroke).collect();
        let id = save_session(&pool, &meta, &summary, &strokes).await.unwrap();

        // Re-analysing keeps the id and replaces the strokes.
        summary.whole.watts = Some(81.0);
        let id2 = save_session(&pool, &meta, &summary, &strokes[..2]).await.unwrap();
        assert_eq!(id, id2);
        let all = list_sessions(&pool).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].meta, meta);
        assert_eq!(all[0].summary, summary);
        assert_eq!(session_strokes(&pool, id).await.unwrap(), strokes[..2].to_vec());

        pool.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn profile_defaults_then_saves() {
        let dir = std::env::temp_dir().join(format!("row-assist-profile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pool = open(&dir.join("t.db")).await.unwrap();

        assert_eq!(get_profile(&pool).await.unwrap(), Profile::default());
        assert_eq!(latest_age(&pool).await.unwrap(), None);
        let p = Profile {
            hr_rest: Some(55),
            hr_max: Some(186),
            sex: Some(Sex::Female),
            drag_target: Some(110),
        };
        set_profile(&pool, &p).await.unwrap();
        set_profile(&pool, &p).await.unwrap();
        assert_eq!(get_profile(&pool).await.unwrap(), p);

        pool.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
