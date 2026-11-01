//! `/api/progress`: the long view, from every row in both sources.

use analysis::progress::{Progress, ProgressRow, fault_halves, progress};
use axum::Json;
use axum::extract::State;
use store::health::day_number;
use store::history::{Entry, entries};

use crate::{Api, ApiError};

fn row(e: &Entry) -> ProgressRow {
    let day = e.day();
    match e {
        Entry::Logbook(r) => ProgressRow {
            day,
            minutes: r.work_time_s / 60.0,
            distance_m: Some(r.work_distance_m as f64),
            watts: r.avg_watts,
            spm: r.stroke_rate,
            hr: r.avg_hr.map(|h| h as f64),
            intervals: r.rest_time_s.is_some_and(|s| s > 0.0),
            from_logbook: true,
            ..Default::default()
        },
        Entry::App(s) => {
            let w = &s.summary.whole;
            ProgressRow {
                day,
                minutes: w.seconds / 60.0,
                distance_m: w.distance_m,
                watts: w.watts,
                spm: w.spm,
                hr: w.hr,
                trimp: s.summary.heart.as_ref().and_then(|h| h.trimp),
                watts_at_20: s.summary.rate_20.as_ref().and_then(|r| r.watts),
                plan: s.meta.plan.clone(),
                faults: s.summary.faults.clone(),
                intervals: false,
                from_logbook: false,
            }
        }
    }
}

pub async fn get(State(api): State<Api>) -> Result<Json<Progress>, ApiError> {
    let pool = &api.pool;
    let today = day_number(chrono::Local::now().date_naive());
    let all = entries(pool, None).await?;
    let rows: Vec<ProgressRow> = all.iter().map(row).collect();

    let mut halves = Vec::new();
    for e in &all {
        if let Entry::App(s) = e
            && s.summary.camera.is_some()
        {
            let strokes = store::sessions::session_strokes(pool, s.id).await?;
            halves.push(fault_halves(&strokes));
        }
    }

    // The same heart profile the CLI uses, for estimating logbook load.
    let profile = store::sessions::get_profile(pool).await?;
    let age = store::sessions::latest_age(pool).await?;
    let recent = store::health::list_days(pool, today - 30).await?;
    let watch_rest = analysis::readiness::zone_rest(&recent, today);
    let heart = profile.heart(age, watch_rest).map(|h| h.0);
    Ok(Json(progress(&rows, &halves, heart.as_ref(), today)))
}
