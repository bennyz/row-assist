//! What a row is rowed as (DESIGN.md §8): one continuous piece, or work and
//! rest intervals that the PM5 times. Every part has its own targets. The
//! coach follows the PM5 from work to rest and back (`stage`), so its calls
//! match the monitor even after a warm-up in the same row.

use serde::{Deserialize, Serialize};

use crate::cues::Targets;
use crate::zones::HeartProfile;

/// The workouts there are: the planner proposes one, the rower may pick
/// another on the row screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// 20 min at rate 20, ~53 W: measures what fitness is left after a gap.
    Reference,
    /// 20–25 min easy, rate 18–20, zone 2 or below.
    Easy,
    /// 30–45 min zone 2, rate 18–20.
    Steady,
    /// Rate steps at matched form: 4 × 5 min at 20 / 22 / 24 / 22.
    RateSteps,
    /// Norwegian 4×4: 4 × 4 min at 85–95% of max heart rate, 3 min easy between.
    FourByFour,
    /// 20 min held at rate 20, best steady power: the benchmark.
    Rate20Test,
    /// 15–20 min easy: better than skipping.
    Minimum,
}

/// Work intervals, as the PM5 is set up for them by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intervals {
    pub count: u32,
    pub work_s: u32,
    pub rest_s: u32,
}

/// Rate steps' work intervals, in order.
pub const RATE_STEPS: [f64; 4] = [20.0, 22.0, 24.0, 22.0];
/// A 4×4 work interval: rate kept down so strokes stay long.
pub const FOUR_BY_FOUR_RATE: (f64, f64) = (24.0, 28.0);
/// The 4×4's ceiling, as a share of max heart rate (not of the reserve).
pub const FOUR_BY_FOUR_HR_TOP: f64 = 0.95;

impl Kind {
    pub const ALL: [Self; 7] = [
        Self::Reference,
        Self::Easy,
        Self::Steady,
        Self::RateSteps,
        Self::FourByFour,
        Self::Rate20Test,
        Self::Minimum,
    ];

    pub fn describe(self) -> &'static str {
        match self {
            Self::Reference => "reference row: 20 min at rate 20, about 53 W",
            Self::Easy => "easy row: rate 18–20, heart rate in zone 2 or below",
            Self::Steady => "steady row: zone 2, rate 18–20",
            Self::RateSteps => "rate steps: 4 × 5 min at rate 20 / 22 / 24 / 22, 2 min easy between, same form at every rate",
            Self::FourByFour => "Norwegian 4×4: 4 × 4 min hard (heart rate 85–95% of max), 3 min easy between",
            Self::Rate20Test => "rate-20 test: 20 min held at exactly 20 spm, best steady power",
            Self::Minimum => "minimum session: easy, rate 18–20",
        }
    }

    /// What the rower calls it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Reference => "reference row",
            Self::Easy => "easy row",
            Self::Steady => "steady row",
            Self::RateSteps => "rate steps",
            Self::FourByFour => "4×4",
            Self::Rate20Test => "rate-20 test",
            Self::Minimum => "minimum session",
        }
    }

    /// The name a recording's header keeps, which the planner reads back:
    /// persisted, so never renamed. A minimum session is rowed as an easy row.
    pub fn name(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Easy | Self::Minimum => "easy",
            Self::Steady => "steady",
            Self::RateSteps => "rate_steps",
            Self::FourByFour => "four_by_four",
            Self::Rate20Test => "rate20",
        }
    }

    /// Back from a header's name; `None` for a row with no plan or an unknown one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }

    /// Hard sessions: low readiness and easy weeks swap them for easy rowing.
    pub fn is_hard(self) -> bool {
        matches!(self, Self::RateSteps | Self::FourByFour | Self::Rate20Test)
    }

    /// The length the planner gives, when picked without a plan; a fixed
    /// workout's is its whole session with a warm-up.
    pub fn minutes(self) -> (u32, u32) {
        match self {
            Self::Reference | Self::Rate20Test => (20, 20),
            Self::Easy => (20, 25),
            Self::Steady => (30, 45),
            Self::RateSteps => (30, 35),
            Self::FourByFour => (35, 40),
            Self::Minimum => (15, 20),
        }
    }

    /// Whether the planner's minutes set the piece's length (else the
    /// workout fixes it).
    pub fn is_fixed(self) -> bool {
        matches!(self, Self::Reference | Self::Rate20Test | Self::RateSteps | Self::FourByFour)
    }

    pub fn intervals(self) -> Option<Intervals> {
        match self {
            Self::RateSteps => Some(Intervals { count: 4, work_s: 300, rest_s: 120 }),
            Self::FourByFour => Some(Intervals { count: 4, work_s: 240, rest_s: 180 }),
            _ => None,
        }
    }

    /// What to set on the PM5 by hand, for an interval workout.
    pub fn pm5_setup(self) -> Option<String> {
        self.intervals().map(|i| {
            format!(
                "Intervals: Time, {} work / {} rest; press Menu after interval {}",
                mmss(i.work_s),
                mmss(i.rest_s),
                i.count
            )
        })
    }

    /// The workout with its targets; `heart` gives the heart-rate ceilings.
    /// A piece's length is the shortest the kind allows: the plan (or the
    /// command line) can set a longer one.
    pub fn workout(self, heart: Option<&HeartProfile>) -> Workout {
        let zone2_top = heart.and_then(|h| h.zone_bpm(2)).map(|(_, top)| top);
        let piece = |minutes: f64, targets: Targets| Workout::Piece {
            minutes: Some(minutes),
            targets,
        };
        let easy = Targets {
            rate: Some((17.5, 20.5)),
            watts: None,
            hr_max: zone2_top,
        };
        match self {
            Self::Reference => piece(
                20.0,
                Targets {
                    rate: Some((19.0, 21.0)),
                    watts: Some((48.0, 58.0)),
                    hr_max: None,
                },
            ),
            Self::Easy | Self::Minimum | Self::Steady => piece(self.minutes().0 as f64, easy),
            Self::Rate20Test => piece(
                20.0,
                Targets {
                    rate: Some((19.5, 20.5)),
                    ..Default::default()
                },
            ),
            Self::RateSteps | Self::FourByFour => {
                let i = self.intervals().expect("an interval workout");
                let work = match self {
                    Self::RateSteps => RATE_STEPS
                        .iter()
                        .map(|&r| Targets {
                            rate: Some((r - 1.0, r + 1.0)),
                            ..Default::default()
                        })
                        .collect(),
                    _ => vec![
                        Targets {
                            rate: Some(FOUR_BY_FOUR_RATE),
                            watts: None,
                            hr_max: heart.map(|h| (h.max * FOUR_BY_FOUR_HR_TOP).round()),
                        };
                        i.count as usize
                    ],
                };
                Workout::Intervals {
                    work_s: i.work_s,
                    rest_s: i.rest_s,
                    work,
                }
            }
        }
    }
}

/// "4:00".
pub fn mmss(s: u32) -> String {
    format!("{}:{:02}", s / 60, s % 60)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Workout {
    /// One continuous piece; `minutes` gives the halfway and time calls.
    Piece { minutes: Option<f64>, targets: Targets },
    /// Work and rest timed by the PM5: one `Targets` per work interval, in
    /// order. Rests have none: paddle easy.
    Intervals { work_s: u32, rest_s: u32, work: Vec<Targets> },
}

impl Workout {
    /// Nothing to coach: no targets and no length.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Piece { minutes: None, targets } if targets.is_empty())
    }

    /// A piece rowed for this long instead; intervals keep their own.
    pub fn set_minutes(&mut self, m: f64) {
        if let Self::Piece { minutes, .. } = self {
            *minutes = Some(m);
        }
    }

    /// Every set of targets in it, to override from the command line.
    pub fn targets_mut(&mut self) -> Vec<&mut Targets> {
        match self {
            Self::Piece { targets, .. } => vec![targets],
            Self::Intervals { work, .. } => work.iter_mut().collect(),
        }
    }
}

/// Where an interval workout set on the PM5 is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Work,
    Rest,
    /// The PM5's workout ended (or was stopped).
    Done,
}

/// The stage from the PM5's workout state code (Concept2's PM CSAFE spec).
/// `None` outside intervals: 0 waiting to begin and 1 a single piece (both
/// seen on hardware), 2 a countdown pause, 13 rearm. The interval codes
/// follow the spec and are still to be seen on hardware: 3 rest, 4–5 work
/// by time or distance, 6–7 rest ending into work, 8–9 work ending into
/// rest, 10–12 the workout ended, stopped or logged.
pub fn stage(code: u8) -> Option<Stage> {
    match code {
        4..=7 => Some(Stage::Work),
        3 | 8 | 9 => Some(Stage::Rest),
        10..=12 => Some(Stage::Done),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_stay_what_old_recordings_say() {
        for k in Kind::ALL {
            let back = Kind::from_name(k.name()).unwrap();
            assert_eq!(back.name(), k.name(), "{k:?}");
        }
        // Persisted before this module existed.
        assert_eq!(Kind::from_name("rate20"), Some(Kind::Rate20Test));
        assert_eq!(Kind::from_name("rate_steps"), Some(Kind::RateSteps));
        assert_eq!(Kind::Minimum.name(), "easy");
        assert_eq!(Kind::from_name("bogus"), None);
    }

    #[test]
    fn four_by_four_is_four_hard_intervals_under_95_percent() {
        let h = HeartProfile::new(60.0, 184.0, None).unwrap();
        let Workout::Intervals { work_s, rest_s, work } = Kind::FourByFour.workout(Some(&h)) else {
            panic!("intervals");
        };
        assert_eq!((work_s, rest_s, work.len()), (240, 180, 4));
        assert!(work.iter().all(|t| t.hr_max == Some(175.0) && t.rate == Some(FOUR_BY_FOUR_RATE)));
        assert_eq!(
            Kind::FourByFour.pm5_setup().as_deref(),
            Some("Intervals: Time, 4:00 work / 3:00 rest; press Menu after interval 4")
        );
        assert!(Kind::FourByFour.is_hard());
    }

    #[test]
    fn rate_steps_step_the_rate() {
        let Workout::Intervals { work, .. } = Kind::RateSteps.workout(None) else {
            panic!("intervals");
        };
        let mids: Vec<f64> = work.iter().map(|t| t.rate.map(|(lo, hi)| (lo + hi) / 2.0).unwrap()).collect();
        assert_eq!(mids, RATE_STEPS);
    }

    #[test]
    fn easy_ceiling_is_top_of_zone_2_and_the_plan_sets_its_length() {
        let h = HeartProfile::new(60.0, 190.0, None).unwrap();
        let mut easy = Kind::Easy.workout(Some(&h));
        easy.set_minutes(25.0);
        let Workout::Piece { minutes, targets } = easy else {
            panic!("a piece");
        };
        assert_eq!((minutes, targets.hr_max), (Some(25.0), Some(151.0)));
        let mut four = Kind::FourByFour.workout(None);
        four.set_minutes(40.0);
        assert_eq!(four, Kind::FourByFour.workout(None), "intervals keep their length");
    }

    #[test]
    fn pm5_states() {
        assert_eq!(stage(1), None, "a single piece");
        assert_eq!(stage(0), None);
        assert_eq!(stage(4), Some(Stage::Work));
        assert_eq!(stage(3), Some(Stage::Rest));
        assert_eq!(stage(8), Some(Stage::Rest), "work ending into rest");
        assert_eq!(stage(6), Some(Stage::Work), "rest ending into work");
        assert_eq!(stage(10), Some(Stage::Done));
    }
}
