//! Reference-row check: has aerobic fitness held up since the last block?
//!
//! EF (watts per bpm) climbs with power, so a reference row is only compared
//! with past steady pieces at similar watts. The bands come from the rower's
//! own history: the mean of the best few comparable pieces is "fitness
//! retained", the mean of the worst few is "start from scratch".

use serde::{Deserialize, Serialize};

use crate::session::Segment;

/// A past steady piece, e.g. from the Concept2 logbook.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Piece {
    pub label: String,
    pub minutes: f64,
    pub watts: f64,
    pub spm: f64,
    pub hr: f64,
}

impl Piece {
    pub fn ef(&self) -> f64 {
        self.watts / self.hr
    }
}

/// Pieces shorter than this don't reach a steady heart rate.
pub const MIN_MINUTES: f64 = 15.0;
/// Past pieces count when their power is within this many watts of today's.
pub const WATTS_WINDOW: f64 = 8.0;
pub const SPM_RANGE: (f64, f64) = (16.0, 22.0);
/// Fewer comparable pieces than this and there's no meaningful band.
pub const MIN_PIECES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ramp {
    /// At or above your best: one easy week, then the normal plan.
    OneWeek,
    /// In between: two weeks.
    TwoWeeks,
    /// At or below your worst: the full 2–3 week ramp.
    Full,
}

impl Ramp {
    pub fn describe(self) -> &'static str {
        match self {
            Self::OneWeek => "fitness held: 1-week ramp, then the normal plan",
            Self::TwoWeeks => "partly held: 2-week ramp",
            Self::Full => "back to base: full 2–3 week ramp",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub ef: f64,
    pub watts: f64,
    /// Mean EF of the best and worst few comparable pieces.
    pub high: f64,
    pub low: f64,
    pub pieces: Vec<Piece>,
    pub ramp: Ramp,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Compared(Comparison),
    /// Today's row isn't a steady reference piece; says why.
    NotSteady(String),
    /// Not enough history at this power.
    NoBaseline { ef: f64, watts: f64, pieces: usize },
}

/// Compare a steady segment (normally the whole session) with past pieces.
pub fn compare(today: &Segment, history: &[Piece]) -> Outcome {
    let minutes = today.seconds / 60.0;
    let (Some(watts), Some(ef), Some(spm)) = (today.watts, today.ef, today.spm) else {
        return Outcome::NotSteady("needs power and heart rate".into());
    };
    if minutes < MIN_MINUTES {
        return Outcome::NotSteady(format!("{minutes:.0} min rowed, needs {MIN_MINUTES:.0}"));
    }
    if !(SPM_RANGE.0..=SPM_RANGE.1).contains(&spm) {
        return Outcome::NotSteady(format!("rate {spm:.1}, needs {}–{}", SPM_RANGE.0, SPM_RANGE.1));
    }

    let mut pieces: Vec<Piece> = history
        .iter()
        .filter(|p| {
            p.minutes >= MIN_MINUTES
                && p.hr > 0.0
                && (p.watts - watts).abs() <= WATTS_WINDOW
                && (SPM_RANGE.0..=SPM_RANGE.1).contains(&p.spm)
        })
        .cloned()
        .collect();
    if pieces.len() < MIN_PIECES {
        return Outcome::NoBaseline {
            ef,
            watts,
            pieces: pieces.len(),
        };
    }
    pieces.sort_by(|a, b| b.ef().total_cmp(&a.ef()));
    let k = (pieces.len() / 2).min(3);
    let mean = |ps: &[Piece]| ps.iter().map(Piece::ef).sum::<f64>() / ps.len() as f64;
    let high = mean(&pieces[..k]);
    let low = mean(&pieces[pieces.len() - k..]);
    let ramp = if ef >= high {
        Ramp::OneWeek
    } else if ef <= low {
        Ramp::Full
    } else {
        Ramp::TwoWeeks
    };
    Outcome::Compared(Comparison {
        ef,
        watts,
        high,
        low,
        pieces,
        ramp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(minutes: f64, watts: f64, spm: f64, hr: f64) -> Piece {
        Piece {
            label: format!("{watts} W"),
            minutes,
            watts,
            spm,
            hr,
        }
    }

    fn today(watts: f64, hr: f64) -> Segment {
        Segment {
            strokes: 400,
            seconds: 1200.0,
            watts: Some(watts),
            spm: Some(20.0),
            hr: Some(hr),
            ef: Some(watts / hr),
            ..Default::default()
        }
    }

    /// Synthetic season: steady pieces at 45–58 W with EF 0.32–0.39, plus
    /// pieces that must not count (too short, too hard, too slow, no HR).
    fn history() -> Vec<Piece> {
        vec![
            piece(30.0, 46.0, 18.0, 130.0),
            piece(16.0, 45.0, 18.0, 131.0),
            piece(18.0, 50.0, 18.0, 138.0),
            piece(35.0, 55.0, 17.0, 154.0),
            piece(50.0, 58.0, 19.0, 150.0),
            piece(40.0, 50.0, 19.0, 134.0),
            piece(40.0, 48.0, 19.0, 149.0),
            piece(42.0, 54.0, 20.0, 152.0),
            piece(10.0, 60.0, 20.0, 158.0), // too short
            piece(20.0, 85.0, 20.0, 160.0), // too hard: EF 0.53
            piece(40.0, 52.0, 13.0, 120.0), // rate too low
            piece(30.0, 50.0, 18.0, 0.0),   // no HR
        ]
    }

    #[test]
    fn bands_come_from_comparable_pieces_only() {
        let Outcome::Compared(c) = compare(&today(52.0, 140.0), &history()) else {
            panic!("expected a comparison");
        };
        assert_eq!(c.pieces.len(), 8);
        assert!((c.high - 0.374).abs() < 0.001, "high {}", c.high);
        assert!((c.low - 0.340).abs() < 0.001, "low {}", c.low);
        assert_eq!(c.ramp, Ramp::TwoWeeks); // 0.371
    }

    #[test]
    fn verdicts() {
        let ramp = |hr| match compare(&today(52.0, hr), &history()) {
            Outcome::Compared(c) => c.ramp,
            other => panic!("{other:?}"),
        };
        assert_eq!(ramp(135.0), Ramp::OneWeek); // 0.385
        assert_eq!(ramp(160.0), Ramp::Full); // 0.325
    }

    #[test]
    fn refuses_unsteady_rows() {
        let mut short = today(52.0, 140.0);
        short.seconds = 600.0;
        assert!(matches!(compare(&short, &history()), Outcome::NotSteady(_)));
        let mut fast = today(52.0, 140.0);
        fast.spm = Some(26.0);
        assert!(matches!(compare(&fast, &history()), Outcome::NotSteady(_)));
        let mut no_hr = today(52.0, 140.0);
        no_hr.ef = None;
        assert!(matches!(compare(&no_hr, &history()), Outcome::NotSteady(_)));
    }

    #[test]
    fn no_baseline_at_an_unusual_power() {
        assert!(matches!(
            compare(&today(120.0, 170.0), &history()),
            Outcome::NoBaseline { pieces: 0, .. }
        ));
    }
}
