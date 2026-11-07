//! `check`: score the fault detectors on a recording with marked blocks.

use std::path::Path;

use analysis::fixtures::{blocks, check};
use anyhow::Result;

use crate::render::clock;

pub fn run(file: &Path) -> Result<()> {
    let (_, events, _) = recording::read_all(file)?;
    let marked = blocks(&events);
    if marked.is_empty() {
        println!(
            "No marked blocks. While rowing, press Clean or a fault under \"Mark faults on purpose…\" on the row screen (or, with rowctl record, type 0 for clean or 1–4 for a fault and Enter)."
        );
        return Ok(());
    }
    let a = analysis::analyze(&events, None);
    let start = a.strokes.first().map_or(marked[0].from_us, |s| s.t_catch_us);
    let at = |us: i64| clock((us - start).max(0) as f64 / 1e6);
    let results = check(&a.strokes, &marked);
    for c in &results {
        let until = c.block.to_us.map(at).unwrap_or_else(|| "end".into());
        let verdict = match (c.passed(), c.block.intended) {
            (true, Some(f)) => format!("PASS: the coach would say \"{}\"", f.cue()),
            (true, None) => "PASS: the coach stays quiet".into(),
            (false, Some(f)) => format!("MISS: {} not caught often enough to cue", f.label()),
            (false, None) => format!(
                "FALSE ALARM: would cue {}",
                c.coach_spoke.iter().map(|f| f.label()).collect::<Vec<_>>().join(", ")
            ),
        };
        println!("{} ({}–{}), {} strokes: {verdict}", c.block.label, at(c.block.from_us), until, c.strokes);
        let flags: Vec<String> = c
            .flagged
            .iter()
            .map(|&(f, _)| format!("{} {:.0}%", f.label(), c.share(f) * 100.0))
            .collect();
        println!("  flagged: {}", flags.join(", "));
    }
    let passed = results.iter().filter(|c| c.passed()).count();
    println!("\n{passed} of {} blocks pass", results.len());
    if a.summary.as_ref().is_some_and(|s| s.camera.is_none()) {
        println!("(No camera in this recording: only rushed recovery can be judged.)");
    }
    Ok(())
}
