//! `profile`: show or change the rower's settings.

use std::path::PathBuf;

use analysis::zones::{Sex, ZONE_FLOORS};
use anyhow::Result;
use clap::{Args, ValueEnum};

use crate::ctx::Ctx;

#[derive(Args, Debug)]
pub struct ProfileArgs {
    /// Resting heart rate, bpm (0 clears it)
    #[arg(long)]
    hr_rest: Option<u16>,
    /// Maximum heart rate, bpm (0 clears it: estimated from age again)
    #[arg(long)]
    hr_max: Option<u16>,
    /// Sets the TRIMP weighting
    #[arg(long, value_enum)]
    sex: Option<SexArg>,
    /// Drag factor to row every session at (0 clears it)
    #[arg(long)]
    drag: Option<u8>,
    #[arg(long)]
    db: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SexArg {
    Male,
    Female,
}

pub async fn run(args: ProfileArgs) -> Result<()> {
    let mut ctx = Ctx::open(args.db).await?;
    let changing = args.hr_rest.is_some() || args.hr_max.is_some() || args.sex.is_some() || args.drag.is_some();
    if changing {
        let p = &mut ctx.profile;
        let nonzero16 = |v: u16| (v > 0).then_some(v);
        if let Some(v) = args.hr_rest {
            p.hr_rest = nonzero16(v);
        }
        if let Some(v) = args.hr_max {
            p.hr_max = nonzero16(v);
        }
        if let Some(s) = args.sex {
            p.sex = Some(match s {
                SexArg::Male => Sex::Male,
                SexArg::Female => Sex::Female,
            });
        }
        if let Some(v) = args.drag {
            p.drag_target = (v > 0).then_some(v);
        }
        if ctx.heart().is_none() {
            anyhow::bail!("HRmax must be at least 40 above HRrest; nothing saved");
        }
        store::sessions::set_profile(&ctx.pool, &ctx.profile).await?;
        println!("Saved.");
    }

    let p = &ctx.profile;
    println!("{}", ctx.heart_note());
    println!(
        "Sex: {}",
        match p.sex {
            Some(Sex::Male) => "male",
            Some(Sex::Female) => "female",
            None => "not set (TRIMP uses the male coefficient)",
        }
    );
    println!(
        "Drag factor target: {}",
        p.drag_target.map(|d| d.to_string()).unwrap_or_else(|| "not set".into())
    );
    if let Some((h, _, _)) = ctx.heart() {
        let zones: Vec<String> = (1..=ZONE_FLOORS.len() as u8)
            .filter_map(|z| h.zone_bpm(z).map(|(lo, hi)| format!("Z{z} {lo:.0}–{hi:.0}")))
            .collect();
        println!("Zones (Karvonen): {}", zones.join("  "));
    }
    if !changing && (p.hr_rest.is_none() || p.hr_max.is_none()) {
        println!("Set with: rowctl profile --hr-rest 58 --hr-max 186 --sex male --drag 110");
    }
    Ok(())
}
