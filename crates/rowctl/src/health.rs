//! `health`: Apple Watch recovery data and today's readiness.

use std::path::PathBuf;

use analysis::readiness::{HRV_BASELINE_DAYS, Level, WEEK, readiness};
use anyhow::{Context, Result};
use clap::Args;
use store::health::{Samples, aggregate, day_date, list_days, token, upsert_days};

use crate::ctx::{Ctx, today};

#[derive(Args, Debug)]
pub struct HealthArgs {
    /// Print the token and address an iPhone Shortcut posts to
    #[arg(long)]
    token: bool,
    /// Import samples from a JSON file in the Shortcut's format
    #[arg(long)]
    import: Option<PathBuf>,
    #[arg(long)]
    db: Option<PathBuf>,
}

pub async fn run(args: HealthArgs) -> Result<()> {
    if args.token {
        let t = token(&store::paths::token_path())?;
        println!("Token: {t}");
        println!("The Shortcut POSTs JSON to http://<this Mac's address>:8080/api/health/daily");
        println!("with the header  Authorization: Bearer {t}");
        println!("Serve on the network for that: rowctl serve --addr 0.0.0.0:8080. Setup: docs/SHORTCUT.md");
        return Ok(());
    }
    let ctx = Ctx::open(args.db).await?;
    if let Some(file) = &args.import {
        let text = std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
        let samples: Samples = serde_json::from_str(&text).context("parse samples")?;
        let days = aggregate(&samples)?;
        upsert_days(&ctx.pool, &days).await?;
        println!("Imported {} days", days.len());
    }
    let today = today();
    let days = list_days(&ctx.pool, today - HRV_BASELINE_DAYS - WEEK).await?;
    if days.is_empty() {
        println!("No Watch data yet. Set up the Shortcut: rowctl health --token");
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
