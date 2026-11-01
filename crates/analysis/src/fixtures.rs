//! Deliberate-fault fixtures (DESIGN.md §5.3, validation): rows where faults
//! are made on purpose and marked as they start, to check that each detector
//! flags its fault and that the coach stays quiet on clean rowing.

use model::{Event, EventKind};
use serde::Serialize;

use crate::cues::{FocusCoach, TechniqueCue};
use crate::strokes::StrokeRecord;
use crate::technique::Fault;

/// Marker label for clean rowing.
pub const CLEAN: &str = "clean";
/// Strokes at the start of a block that don't count: changing technique on
/// purpose takes a stroke or two.
pub const SETTLE_STROKES: usize = 2;

/// What a marker label means: `Some(None)` clean, `Some(Some(f))` rowing `f`
/// on purpose, `None` a free-form note.
pub fn intent(label: &str) -> Option<Option<Fault>> {
    let l = label.trim().to_lowercase();
    if l == CLEAN {
        return Some(None);
    }
    Fault::ALL.into_iter().find(|f| f.label() == l).map(Some)
}

/// From one marker to the next.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Block {
    pub label: String,
    pub intended: Option<Fault>,
    pub from_us: i64,
    /// `None` for the last block (to the end of the recording).
    pub to_us: Option<i64>,
}

/// Blocks from the recording's markers; free-form notes don't start one.
pub fn blocks<'a>(events: impl IntoIterator<Item = &'a Event>) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for ev in events {
        if let EventKind::Marker { label } = &ev.kind
            && let Some(intended) = intent(label)
        {
            if let Some(prev) = out.last_mut() {
                prev.to_us = Some(ev.t_us);
            }
            out.push(Block {
                label: label.trim().to_lowercase(),
                intended,
                from_us: ev.t_us,
                to_us: None,
            });
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BlockCheck {
    pub block: Block,
    /// Strokes judged (after the settling strokes).
    pub strokes: u32,
    /// How many strokes each fault was flagged on.
    pub flagged: Vec<(Fault, u32)>,
    /// Faults the focus coach would have spoken about, had it been the focus.
    pub coach_spoke: Vec<Fault>,
}

impl BlockCheck {
    /// A fault block passes when the coach would speak about that fault; a
    /// clean block when it would say nothing.
    pub fn passed(&self) -> bool {
        match self.block.intended {
            Some(f) => self.coach_spoke.contains(&f),
            None => self.coach_spoke.is_empty(),
        }
    }

    pub fn share(&self, fault: Fault) -> f64 {
        let n = self.flagged.iter().find(|(f, _)| *f == fault).map_or(0, |x| x.1);
        n as f64 / self.strokes.max(1) as f64
    }
}

pub fn check(strokes: &[StrokeRecord], blocks: &[Block]) -> Vec<BlockCheck> {
    blocks
        .iter()
        .map(|b| {
            let inside: Vec<&StrokeRecord> = strokes
                .iter()
                .filter(|s| s.t_catch_us >= b.from_us && b.to_us.is_none_or(|t| s.t_catch_us < t))
                .skip(SETTLE_STROKES)
                .collect();
            let camera = inside.iter().any(|s| s.pose.is_some());
            let judged = Fault::ALL.into_iter().filter(|f| camera || !f.needs_camera());
            let flagged = judged
                .clone()
                .map(|f| (f, inside.iter().filter(|s| s.faults.iter().any(|h| h.fault == f)).count() as u32))
                .collect();
            let coach_spoke = judged
                .filter(|&f| {
                    let mut coach = FocusCoach::new(f);
                    inside
                        .iter()
                        .any(|s| matches!(coach.stroke(s), Some(TechniqueCue::Fix { .. })))
                })
                .collect();
            BlockCheck {
                block: b.clone(),
                strokes: inside.len() as u32,
                flagged,
                coach_spoke,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use devices::sim::{SimConfig, generate};
    use devices::sim_pose::{PoseSim, SimFault};

    use super::*;
    use crate::session::tests::T0;

    fn marker(t_us: i64, label: &str) -> Event {
        Event {
            t_us,
            kind: EventKind::Marker { label: label.into() },
        }
    }

    /// A clean minute, then a minute of early arms, as one recording.
    fn fixture() -> Vec<Event> {
        let part = |fault, start| {
            let cfg = SimConfig {
                pose: Some(PoseSim {
                    fault,
                    ..Default::default()
                }),
                ..Default::default()
            };
            generate(cfg, start, Duration::from_secs(60))
        };
        let mut events = vec![marker(T0, "clean")];
        events.extend(part(None, T0));
        events.push(marker(T0 + 60_000_000, "Early arms "));
        events.extend(part(Some(SimFault::EarlyArms), T0 + 60_000_000));
        events.push(marker(T0 + 61_000_000, "felt good")); // a note, not a block
        events
    }

    #[test]
    fn labels() {
        assert_eq!(intent("clean"), Some(None));
        assert_eq!(intent(" Shooting the slide"), Some(Some(Fault::ShootingSlide)));
        assert_eq!(intent("tired"), None);
    }

    #[test]
    fn the_coach_speaks_on_the_fault_and_not_on_clean_rowing() {
        let events = fixture();
        let a = crate::analyze(&events, None);
        let b = blocks(&events);
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].to_us, b[1].intended), (Some(T0 + 60_000_000), Some(Fault::EarlyArms)));

        let checks = check(&a.strokes, &b);
        let (clean, arms) = (&checks[0], &checks[1]);
        assert!(clean.passed(), "clean: {clean:?}");
        assert_eq!(clean.share(Fault::EarlyArms), 0.0);
        assert!(arms.passed(), "early arms: {arms:?}");
        assert!(arms.share(Fault::EarlyArms) > 0.9);
        assert!(arms.strokes >= 15);
    }
}
