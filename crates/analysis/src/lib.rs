//! Pure analysis over the event stream. No I/O: everything here is a function
//! of events, so it runs the same live, on replay, and in tests.

pub mod cues;
pub mod metrics;
pub mod pose;
pub mod reference;
pub mod session;
pub mod strokes;
pub mod technique;
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
    }
    Analysis {
        summary,
        orphan_points: asm.orphan_points(),
        strokes,
    }
}
