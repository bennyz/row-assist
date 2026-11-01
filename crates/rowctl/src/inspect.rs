//! `dump`, `replay` and `curves`: read recordings back.

use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Duration;

use analysis::strokes::drive_curves;
use anyhow::{Result, bail};
use recording::Reader;

use crate::render::stroke_line;

const TRUNCATED: &str = "note: recording ends mid-stream (the writer didn't finish), events up to the last flush are shown";

pub fn dump(file: &Path) -> Result<()> {
    let mut reader = Reader::open(file)?;
    let mut out = BufWriter::new(std::io::stdout().lock());
    writeln!(out, "{{\"header\":{}}}", serde_json::to_string(reader.header())?)?;
    for ev in reader.by_ref() {
        writeln!(out, "{}", serde_json::to_string(&ev?)?)?;
    }
    out.flush()?;
    if reader.truncated() {
        eprintln!("{TRUNCATED}");
    }
    Ok(())
}

pub async fn replay(file: &Path, speed: f64) -> Result<()> {
    if speed.is_nan() || speed <= 0.0 {
        bail!("--speed must be positive");
    }
    let (_, events, truncated) = recording::read_all(file)?;
    let mut prev = events.first().map(|e| e.t_us);
    for ev in &events {
        if let Some(p) = prev {
            let dt = (ev.t_us - p).max(0) as f64 / 1e6 / speed;
            if dt > 0.0 {
                tokio::time::sleep(Duration::from_secs_f64(dt)).await;
            }
        }
        prev = Some(ev.t_us);
        println!("{}", serde_json::to_string(ev)?);
    }
    if truncated {
        eprintln!("{TRUNCATED}");
    }
    Ok(())
}

pub fn curves(file: &Path) -> Result<()> {
    let (header, events, truncated) = recording::read_all(file)?;
    let (curves, orphans) = drive_curves(&events);
    for c in &curves {
        println!("{}", stroke_line(c, header.started_at_us));
    }

    // A drive the recording cut off mid-way would skew the means.
    let complete: Vec<_> = curves.iter().filter(|c| c.drive_end_us.is_some()).collect();
    let cut_off = curves.len() - complete.len();
    let mean = |xs: Vec<f64>| (!xs.is_empty()).then(|| xs.iter().sum::<f64>() / xs.len() as f64);
    let drive = mean(complete.iter().filter_map(|c| c.drive_ms()).collect());
    let recovery = mean(complete.iter().filter_map(|c| c.recovery_ms()).collect());
    let ratio = mean(complete.iter().filter_map(|c| c.ratio()).collect());
    let peak = mean(complete.iter().filter_map(|c| c.peak_lbf()).map(f64::from).collect());
    let pts = mean(complete.iter().map(|c| c.points.len() as f64).collect());
    let f = |v: Option<f64>, d: usize| v.map(|v| format!("{v:.d$}")).unwrap_or_else(|| "–".into());

    println!();
    println!(
        "{} strokes{}{}  mean drive {} ms  recovery {} ms  ratio {}  peak {} lbf  {} pts/stroke",
        complete.len(),
        if cut_off > 0 { format!(" (+{cut_off} cut off)") } else { String::new() },
        if header.simulated { ", simulated" } else { "" },
        f(drive, 0),
        f(recovery, 0),
        f(ratio, 2),
        f(peak, 0),
        f(pts, 1),
    );
    if orphans > 0 {
        println!("{orphans} force samples arrived outside any drive");
    }
    if curves.is_empty() {
        println!("No drives found: was the PM5 connected and the flywheel spinning?");
    }
    if truncated {
        eprintln!("{TRUNCATED}");
    }
    Ok(())
}
