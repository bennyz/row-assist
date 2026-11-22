//! What a row is rowed as (DESIGN.md §8): one continuous piece, or work and
//! rest intervals that the PM5 times. Every part has its own targets. The
//! coach follows the PM5 from work to rest and back (`stage`), so its calls
//! match the monitor even after a warm-up in the same row.

use model::{Event, EventKind};
use serde::{Deserialize, Serialize};

use crate::cues::Targets;
use crate::session::WorkInterval;
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

/// The 4×4's heart-rate ceiling for this rower, bpm.
pub fn four_by_four_hr_top(heart: &HeartProfile) -> f64 {
    (heart.max * FOUR_BY_FOUR_HR_TOP).round()
}

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
                            hr_max: heart.map(four_by_four_hr_top),
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

    /// Power for each work interval (none for a piece): a band around each.
    pub fn set_power(&mut self, watts: &[f64]) {
        if let Self::Intervals { work, .. } = self {
            for (t, &w) in work.iter_mut().zip(watts) {
                t.watts = Some(((w * (1.0 - POWER_BAND)).round(), (w * (1.0 + POWER_BAND)).round()));
            }
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

/// A work interval's power band: this much either side of its target.
pub const POWER_BAND: f64 = 0.04;
/// Held all the way: the next one goes up this much.
pub const STEP_UP: f64 = 0.03;
/// The last work interval within this much of the first: no fade.
pub const FADE: f64 = 0.03;

/// The next interval workout's power, from how the last one of the same
/// kind went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progression {
    /// For each work interval, in order.
    pub watts: Vec<f64>,
    /// Why, in a line: what last time did and what that makes this time.
    pub note: String,
}

/// Each work interval's power for next time: what the last one held, and
/// 3% more when it was held all the way. For the 4×4, held means every
/// interval done, the last within 3% of the first, and heart rate never over
/// the ceiling (`hr_top`): the same power for all four. For rate steps it
/// means every step done at its rate, and each step keeps its own power.
/// `None` with nothing complete to go on: the first one is by heart rate
/// (the 4×4) or by rate alone (rate steps).
pub fn progression(kind: Kind, last: &[WorkInterval], hr_top: Option<f64>) -> Option<Progression> {
    let i = kind.intervals()?;
    let watts: Vec<f64> = last.iter().filter_map(|w| w.seg.watts).collect();
    if last.len() < i.count as usize || watts.len() < i.count as usize {
        return None;
    }
    let watts = &watts[..i.count as usize];
    let (first, end) = (watts[0], watts[watts.len() - 1]);
    let w = |x: f64| format!("{x:.0} W");
    match kind {
        Kind::FourByFour => {
            let mean = watts.iter().sum::<f64>() / watts.len() as f64;
            let peak = last.iter().filter_map(|w| w.hr_peak).max_by(f64::total_cmp);
            let (next, why) = if end < first * (1.0 - FADE) {
                (mean, format!("faded from {} to {}", w(first), w(end)))
            } else if let (Some(top), Some(p)) = (hr_top, peak)
                && p > top
            {
                (mean, format!("heart rate reached {p:.0}, over {top:.0}"))
            } else {
                (mean * (1.0 + STEP_UP), "held all four".to_string())
            };
            let next = next.round();
            Some(Progression {
                watts: vec![next; i.count as usize],
                note: format!("Last 4×4: {} average, {why}: {} this time.", w(mean), w(next)),
            })
        }
        Kind::RateSteps => {
            let on_rate = last.iter().zip(RATE_STEPS).all(|(l, r)| l.seg.spm.is_some_and(|s| (s - r).abs() <= 1.0));
            let k = if on_rate { 1.0 + STEP_UP } else { 1.0 };
            let next: Vec<f64> = watts.iter().map(|x| (x * k).round()).collect();
            let list = |v: &[f64]| v.iter().map(|x| format!("{x:.0}")).collect::<Vec<_>>().join(" / ");
            let why = if on_rate { "every step at its rate" } else { "not every step at its rate" };
            Some(Progression {
                note: format!("Last rate steps: {} W, {why}: {} W this time.", list(watts), list(&next)),
                watts: next,
            })
        }
        _ => None,
    }
}

/// When each work interval ran, from the PM5's workout state in a
/// recording's events: from the first poll in work to the first one out of
/// it (or the recording's end).
pub fn work_windows(events: &[Event]) -> Vec<(i64, i64)> {
    let mut windows = Vec::new();
    let mut open: Option<i64> = None;
    for ev in events {
        let EventKind::Pm5Status(s) = &ev.kind else { continue };
        let Some(code) = s.workout_state else { continue };
        match (stage(code) == Some(Stage::Work), open) {
            (true, None) => open = Some(ev.t_us),
            (false, Some(from)) => {
                windows.push((from, ev.t_us));
                open = None;
            }
            _ => {}
        }
    }
    if let (Some(from), Some(last)) = (open, events.last()) {
        windows.push((from, last.t_us));
    }
    windows
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

    fn done(watts: &[f64], spm: f64, hr_peak: f64) -> Vec<WorkInterval> {
        watts
            .iter()
            .map(|&w| WorkInterval {
                seg: crate::session::Segment {
                    watts: Some(w),
                    spm: Some(spm),
                    ..Default::default()
                },
                hr_peak: Some(hr_peak),
            })
            .collect()
    }

    #[test]
    fn a_four_by_four_held_all_the_way_goes_up_3_percent() {
        let p = progression(Kind::FourByFour, &done(&[110.0, 108.0, 109.0, 109.0], 26.0, 170.0), Some(174.0)).unwrap();
        assert_eq!(p.watts, vec![112.0; 4]);
        assert_eq!(p.note, "Last 4×4: 109 W average, held all four: 112 W this time.");
    }

    #[test]
    fn a_four_by_four_that_faded_or_went_over_repeats() {
        let faded = progression(Kind::FourByFour, &done(&[120.0, 112.0, 105.0, 100.0], 26.0, 170.0), Some(174.0)).unwrap();
        assert_eq!(faded.watts, vec![109.0; 4]);
        assert!(faded.note.contains("faded from 120 W to 100 W"), "{}", faded.note);
        let over = progression(Kind::FourByFour, &done(&[110.0; 4], 26.0, 178.0), Some(174.0)).unwrap();
        assert_eq!(over.watts, vec![110.0; 4]);
        assert!(over.note.contains("over 174"));
        assert_eq!(progression(Kind::FourByFour, &done(&[110.0; 3], 26.0, 160.0), Some(174.0)), None, "stopped after three");
        assert_eq!(progression(Kind::Steady, &done(&[110.0; 4], 26.0, 160.0), None), None);
    }

    #[test]
    fn rate_steps_keep_each_step_s_power_and_go_up_when_on_rate() {
        let mut last = done(&[80.0, 88.0, 95.0, 90.0], 0.0, 150.0);
        for (l, r) in last.iter_mut().zip(RATE_STEPS) {
            l.seg.spm = Some(r + 0.4);
        }
        let p = progression(Kind::RateSteps, &last, None).unwrap();
        assert_eq!(p.watts, vec![82.0, 91.0, 98.0, 93.0]);
        last[2].seg.spm = Some(26.5);
        assert_eq!(progression(Kind::RateSteps, &last, None).unwrap().watts, vec![80.0, 88.0, 95.0, 90.0]);
    }

    #[test]
    fn the_power_targets_go_into_the_work_intervals() {
        let mut w = Kind::FourByFour.workout(None);
        w.set_power(&[110.0; 4]);
        let Workout::Intervals { work, .. } = w else { panic!() };
        assert!(work.iter().all(|t| t.watts == Some((106.0, 114.0))));
    }

    #[test]
    fn work_windows_follow_the_pm5() {
        let poll = |sec: i64, code: u8| Event {
            t_us: sec * 1_000_000,
            kind: EventKind::Pm5Status(model::Pm5Status {
                workout_state: Some(code),
                ..Default::default()
            }),
        };
        let events = [poll(0, 0), poll(1, 4), poll(240, 4), poll(241, 3), poll(420, 6), poll(421, 4), poll(700, 4)];
        assert_eq!(work_windows(&events), vec![(1_000_000, 241_000_000), (420_000_000, 700_000_000)]);
        assert!(work_windows(&[poll(0, 1), poll(5, 1)]).is_empty(), "a single piece has none");
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
