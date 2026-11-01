//! `plan`: what to row today, why, and the command to row it.

use std::path::PathBuf;

use analysis::planner::{Kind, Phase, propose};
use analysis::readiness::{HRV_BASELINE_DAYS, WEEK, readiness};
use anyhow::Result;
use clap::Args;

use crate::ctx::{Ctx, today};

#[derive(Args, Debug)]
pub struct PlanArgs {
    /// You'll row with the camera (so camera faults can be the focus)
    #[arg(long)]
    camera: bool,
    #[arg(long)]
    db: Option<PathBuf>,
}

pub async fn run(args: PlanArgs) -> Result<()> {
    let ctx = Ctx::open(args.db).await?;
    let today = today();
    let health = store::health::list_days(&ctx.pool, today - HRV_BASELINE_DAYS - WEEK).await?;
    let ready = readiness(&health, today);
    let focus = ctx.auto_focus(args.camera).await.map(|f| f.0);
    let rows = ctx.rows().await?;
    let p = propose(&rows, today, ready.level, focus);

    let (lo, hi) = p.minutes;
    // The reference row and the test are fixed 20-minute pieces; their description says so.
    let length = match p.kind {
        Kind::Reference | Kind::Rate20Test => String::new(),
        _ if lo == hi => format!(", {lo} min"),
        _ => format!(", {lo}–{hi} min"),
    };
    println!("Today: {}{length}", p.kind.describe());
    let phase = match p.phase {
        Phase::Restart => "restart".to_string(),
        Phase::Comeback { week, of } => format!("comeback ramp, week {week} of {of}"),
        Phase::EasyWeek => "easy week".into(),
        Phase::Build { week } => format!("build, week {week}"),
    };
    println!("  {phase}; {} of {} rows in the last 7 days", p.rows_this_week, p.target_per_week);
    for w in &p.why {
        println!("  · {w}");
    }
    println!("  · {}", crate::health::line(&ctx).await?);
    if let Some(f) = p.focus {
        println!("  Focus: {} (\"{}\")", f.label(), f.cue());
    }

    let preset = match p.kind {
        Kind::Reference => "reference",
        Kind::Easy | Kind::Minimum => "easy",
        Kind::Steady => "steady",
        Kind::Rate20Test => "rate20",
        Kind::RateSteps => "rate-steps",
    };
    let mut cmd = format!("rowctl record --plan {preset} --speak");
    if !matches!(p.kind, Kind::Reference | Kind::Rate20Test | Kind::RateSteps) {
        cmd += &format!(" --minutes {hi}");
    }
    if let Some(f) = p.focus {
        cmd += &format!(" --focus {}", crate::live::FocusArg::flag(f));
        if f.needs_camera() {
            cmd += " --camera";
        }
    }
    if p.kind == Kind::RateSteps {
        println!("\nSet on the PM5: intervals 4 × 5:00 work / 2:00 rest; rate 20, 22, 24, 22.");
    }
    println!("\nRow it with: {cmd}");
    Ok(())
}
