//! `simulate`: write a whole simulated session to a recording, instantly.
//! For trying analysis on long rows and making test fixtures.

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use devices::sim::{SimConfig, generate};
use model::Clock;
use recording::{FileWriter, Header};

use crate::plan::Plan;

pub struct SimArgs {
    pub minutes: f64,
    pub watts: f64,
    pub spm: f64,
    pub hr: (f64, f64),
    pub plan: Option<Plan>,
    pub note: Option<String>,
}

pub fn run(out: &Path, a: SimArgs) -> Result<()> {
    let cfg = SimConfig {
        spm: a.spm,
        watts: a.watts,
        hr_start: a.hr.0,
        hr_end: a.hr.1,
        ..Default::default()
    };
    let duration = Duration::from_secs_f64(a.minutes * 60.0);
    // Stamp it as if it ended just now.
    let start_us = Clock::start().now_us() - duration.as_micros() as i64;
    let mut header = Header::new(start_us, concat!("rowctl ", env!("CARGO_PKG_VERSION"), " simulate"));
    header.simulated = true;
    header.note = a.note;
    header.plan = a.plan.map(|p| p.name().to_owned());

    let mut w = FileWriter::create(out, &header)?;
    for ev in generate(cfg, start_us, duration) {
        w.write(&ev)?;
    }
    let events = w.events();
    w.finish()?;
    println!("Wrote {events} simulated events ({} min) to {}", a.minutes, out.display());
    Ok(())
}
