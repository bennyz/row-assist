//! `health`: Apple Watch recovery data and today's readiness.

use std::path::PathBuf;

use analysis::readiness::{HRV_BASELINE_DAYS, Level, WEEK, readiness};
use anyhow::{Context, Result};
use clap::Args;
use store::health::{day_date, list_days, set_source, source};
use store::paths::health_source_path;

use crate::ctx::{Ctx, today};

#[derive(Args, Debug)]
pub struct HealthArgs {
    /// Fetch Watch data from here from now on: your health link on the home
    /// server, as nutridag's /health command gives it. Restart `rowctl serve`
    /// (systemctl --user restart row-assist) for it to use the new link
    #[arg(long, value_name = "LINK")]
    source: Option<String>,
    #[arg(long)]
    db: Option<PathBuf>,
}

pub async fn run(args: HealthArgs) -> Result<()> {
    let ctx = Ctx::open(args.db).await?;
    if let Some(link) = &args.source {
        // A link that doesn't work is not kept.
        let synced = server::health::sync(&ctx.pool, link.trim().trim_end_matches('/'))
            .await
            .context("that link didn't give your days")?;
        set_source(&health_source_path(), link)?;
        println!("Link saved. It gave {} days.", synced.days);
    } else if let Some(link) = source(&health_source_path())? {
        // The same fetch `rowctl serve` makes, here so a failure shows.
        match server::health::sync(&ctx.pool, &link).await {
            Ok(synced) => println!("The home server has {} days of yours.", synced.days),
            Err(e) => println!("Couldn't fetch from the home server ({e:#}); showing what was fetched before."),
        }
    } else {
        println!("No source set: rowctl health --source <your link from nutridag's /health>");
    }
    let today = today();
    let days = list_days(&ctx.pool, today - HRV_BASELINE_DAYS - WEEK).await?;
    if days.is_empty() {
        println!("No Watch data yet.");
        return Ok(());
    }
    println!("{}", line(&ctx).await?);
    println!("\n{:<10} {:>6} {:>7} {:>6}", "day", "HRV", "rest HR", "sleep");
    let fmt = |v: Option<f64>, d: usize| v.map(|v| format!("{v:.d$}")).unwrap_or_else(|| "–".into());
    for d in days.iter().filter(|d| d.day > today - 2 * WEEK) {
        println!("{:<10} {:>6} {:>7} {:>6}", day_date(d.day), fmt(d.hrv_ms, 0), fmt(d.resting_hr, 0), fmt(d.sleep_h, 1));
    }
    println!("\nZones use {}", ctx.heart_note());
    Ok(())
}

/// One line for the start of a session: "Readiness: amber — … → easy only".
pub async fn line(ctx: &Ctx) -> Result<String> {
    let today = today();
    let days = list_days(&ctx.pool, today - HRV_BASELINE_DAYS - WEEK).await?;
    let r = readiness(&days, today);
    let label = match r.level {
        Level::Unknown => "unknown",
        Level::Green => "green",
        Level::Amber => "amber",
        Level::Red => "red",
    };
    let why = if r.reasons.is_empty() { String::new() } else { format!(" ({})", r.reasons.join("; ")) };
    Ok(format!("Readiness {label}{why}: {}", r.level.advice()))
}
