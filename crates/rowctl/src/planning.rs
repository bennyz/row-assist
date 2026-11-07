//! `plan`: what to row today, why, and the command to row it. The same plan
//! sets up the rows `rowctl serve` records.

use std::path::PathBuf;

use analysis::planner::{Kind, Phase, Proposal, propose};
use analysis::readiness::{HRV_BASELINE_DAYS, Readiness, WEEK, readiness};
use anyhow::Result;
use clap::Args;

use crate::ctx::{Ctx, today};
use crate::live::Setup;
use crate::plan::Plan;

#[derive(Args, Debug)]
pub struct PlanArgs {
    /// You'll row with the camera (so camera faults can be the focus)
    #[arg(long)]
    camera: bool,
    #[arg(long)]
    db: Option<PathBuf>,
}

/// The planner's proposal for `day`, and the readiness it used.
pub async fn today_plan(ctx: &Ctx, camera: bool, day: i64) -> Result<(Proposal, Readiness)> {
    let health = store::health::list_days(&ctx.pool, day - HRV_BASELINE_DAYS - WEEK).await?;
    let ready = readiness(&health, day);
    let focus = ctx.auto_focus(camera).await.map(|f| f.0);
    let rows = ctx.rows().await?;
    Ok((propose(&rows, day, ready.level, focus), ready))
}

/// The row a proposal sets up: the same preset, length and focus as its
/// `rowctl record` command.
pub fn setup_for(p: &Proposal) -> Setup {
    let plan = match p.kind {
        Kind::Reference => Plan::Reference,
        Kind::Easy | Kind::Minimum => Plan::Easy,
        Kind::Steady => Plan::Steady,
        Kind::Rate20Test => Plan::Rate20,
        Kind::RateSteps => Plan::RateSteps,
    };
    let fixed = matches!(p.kind, Kind::Reference | Kind::Rate20Test | Kind::RateSteps);
    Setup {
        plan: Some(plan),
        minutes: (!fixed).then_some(p.minutes.1 as f64),
        focus: p.focus,
        ..Default::default()
    }
}

pub async fn run(args: PlanArgs) -> Result<()> {
    let ctx = Ctx::open(args.db).await?;
    let (p, _) = today_plan(&ctx, args.camera, today()).await?;

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

    if let Some(setup) = p.pm5_setup() {
        println!("\nSet on the PM5: {setup}.");
    }
    println!("\nRow it with: {}", p.command());
    Ok(())
}

#[cfg(test)]
mod tests {
    use analysis::technique::Fault;

    /// The planner's commands must parse: its focus names match the CLI's.
    #[test]
    fn focus_flags_match_the_cli() {
        for f in Fault::ALL {
            assert_eq!(crate::live::FocusArg::flag(f), f.flag());
        }
    }

    /// `serve` rows what `plan` tells you to type.
    #[test]
    fn served_rows_match_the_plan_command() {
        use analysis::planner::{Kind, Phase, Proposal};
        use clap::ValueEnum;
        let kinds = [Kind::Reference, Kind::Easy, Kind::Steady, Kind::RateSteps, Kind::Rate20Test, Kind::Minimum];
        for kind in kinds {
            let p = Proposal {
                kind,
                minutes: (20, 25),
                phase: Phase::Restart,
                focus: Some(Fault::EarlyArms),
                rows_this_week: 0,
                target_per_week: 3,
                why: vec![],
            };
            let s = super::setup_for(&p);
            let preset = s.plan.unwrap().to_possible_value().unwrap();
            let cmd = p.command();
            assert!(cmd.contains(&format!("--plan {} ", preset.get_name())), "{kind:?}: {cmd}");
            assert_eq!(s.minutes.is_some(), cmd.contains("--minutes 25"), "{kind:?}: {cmd}");
            assert_eq!(s.focus, Some(Fault::EarlyArms));
        }
    }
}
