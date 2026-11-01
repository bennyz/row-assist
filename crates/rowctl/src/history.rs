//! `import-c2` and `history`: the Concept2 logbook baseline.

use std::fs::File;
use std::path::{Path, PathBuf};

use analysis::metrics::{efficiency_factor, joules_per_stroke, metres_per_stroke};
use anyhow::{Context, Result};
use chrono::NaiveDate;

/// Pieces shorter than this are false starts or warm-up fragments.
const MIN_PIECE_S: f64 = 120.0;

pub async fn import(file: &Path, db: Option<PathBuf>) -> Result<()> {
    let csv = File::open(file).with_context(|| format!("open {}", file.display()))?;
    let rows = store::concept2::parse_csv(csv)?;
    let path = db.unwrap_or_else(store::paths::db_path);
    let pool = store::db::open(&path).await?;
    let new = store::db::upsert_concept2(&pool, &rows).await?;
    println!("Imported {} results ({new} new) into {}", rows.len(), path.display());
    Ok(())
}

pub async fn show(db: Option<PathBuf>) -> Result<()> {
    let path = db.unwrap_or_else(store::paths::db_path);
    let pool = store::db::open(&path).await?;
    let rows = store::db::list_concept2(&pool).await?;
    if rows.is_empty() {
        println!("No history yet. Import a logbook export with: rowctl import-c2 <file.csv>");
        return Ok(());
    }

    println!(
        "{:<10} {:>5} {:>5} {:>3} {:>4} {:>4} {:>4} {:>6} {:>6} {:>5} {:>4}",
        "date", "min", "m", "spm", "W", "HR", "DF", "m/str", "J/str", "W/HR", "gap"
    );
    let num = |v: Option<f64>, d: usize| v.map(|v| format!("{v:.d$}")).unwrap_or_else(|| "–".into());
    let int = |v: Option<i64>| v.map(|v| v.to_string()).unwrap_or_else(|| "–".into());

    let mut prev: Option<NaiveDate> = None;
    let mut longest: Option<(i64, NaiveDate, NaiveDate)> = None;
    let mut hidden = 0;
    let mut shown = 0;
    for r in &rows {
        if r.work_time_s < MIN_PIECE_S {
            hidden += 1;
            continue;
        }
        shown += 1;
        let date = r.started_at.date();
        let gap = prev.map(|p| (date - p).num_days());
        if let (Some(g), Some(p)) = (gap, prev)
            && longest.is_none_or(|(l, _, _)| g > l)
        {
            longest = Some((g, p, date));
        }
        prev = Some(date);

        let distance = (r.work_distance_m + r.rest_distance_m.unwrap_or(0)) as f64;
        let mps = r.stroke_count.and_then(|n| metres_per_stroke(distance, n as f64));
        let jps = r.avg_watts.zip(r.stroke_rate).and_then(|(w, spm)| joules_per_stroke(w, spm));
        let ef = r.avg_watts.zip(r.avg_hr).and_then(|(w, hr)| efficiency_factor(w, hr as f64));
        println!(
            "{:<10} {:>5.1} {:>5} {:>3} {:>4} {:>4} {:>4} {:>6} {:>6} {:>5} {:>4}",
            date,
            r.work_time_s / 60.0,
            r.work_distance_m,
            num(r.stroke_rate, 0),
            num(r.avg_watts, 0),
            int(r.avg_hr),
            int(r.drag_factor),
            num(mps, 2),
            num(jps, 0),
            num(ef, 2),
            gap.map(|g| g.to_string()).unwrap_or_default(),
        );
    }

    println!();
    print!("{shown} pieces");
    if hidden > 0 {
        print!(" ({hidden} under {:.0} min hidden)", MIN_PIECE_S / 60.0);
    }
    println!();
    if let Some((days, from, to)) = longest {
        println!("Longest gap: {days} days ({from} → {to})");
    }
    if let Some(last) = prev {
        let today = chrono::Local::now().date_naive();
        println!("Last row: {last}, {} days ago", (today - last).num_days());
    }
    Ok(())
}
