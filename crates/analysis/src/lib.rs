//! Pure analysis over the event stream. No I/O: everything here is a function
//! of events, so it runs the same live, on replay, and in tests.

pub mod cost;
pub mod cues;
pub mod fixtures;
pub mod ideal;
pub mod metrics;
pub mod planner;
pub mod progress;
pub mod pose;
pub mod readiness;
pub mod reference;
pub mod session;
pub mod strokes;
pub mod technique;
pub mod workout;
pub mod zones;

use model::Event;

/// Everything derived from one recording.
#[derive(Debug, Clone)]
pub struct Analysis {
    pub strokes: Vec<strokes::StrokeRecord>,
    pub summary: Option<session::Summary>,
    /// Force samples that arrived outside any drive.
    pub orphan_points: usize,
}

pub fn analyze(events: &[Event], profile: Option<&zones::HeartProfile>) -> Analysis {
    let mut asm = strokes::StrokeAssembler::new();
    let mut strokes: Vec<_> = events.iter().filter_map(|ev| asm.push(ev)).collect();
    strokes.extend(asm.finish());
    let camera = technique::apply(&mut strokes, events);
    let hearts = zones::heart_samples(events);
    let mut summary = session::summarize(&strokes, &hearts, profile);
    if let Some(s) = &mut summary {
        s.camera = camera;
        let windows = workout::work_windows(events);
        s.work = session::work_intervals(&strokes, &windows);
        s.fault_costs = cost::fault_costs(&strokes, &windows);
        // A piece's drift is the piece's own: a warm-up before Go, at
        // another power, would read as drift.
        if let Some((go, end)) = workout::piece_window(events) {
            let piece = &strokes[strokes.partition_point(|r| r.t_catch_us < go)..strokes.partition_point(|r| r.t_catch_us < end)];
            let d = session::drift(piece);
            (s.first_half, s.second_half, s.decoupling_pct) = (d.first_half, d.second_half, d.pct);
        }
    }
    Analysis {
        summary,
        orphan_points: asm.orphan_points(),
        strokes,
    }
}
