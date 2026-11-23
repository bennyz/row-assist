//! What a row is rowed as (DESIGN.md §8): one continuous piece, or work and
//! rest intervals that the PM5 times. Every part has its own targets. The
//! coach follows the PM5 from work to rest and back (`stage`), so its calls
//! match the monitor even after a warm-up in the same row.

use model::{Event, EventKind, Pm5Status, Stage};
use serde::{Deserialize, Serialize};

use crate::cues::{self, PACE_BAND, PACE_BAND_MIN_W, Targets};
use crate::session::{Summary, WorkInterval};
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

/// An interval workout's shape: work intervals and the rests between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intervals {
    pub count: u32,
    pub work_s: u32,
    pub rest_s: u32,
}

/// Rate steps' work intervals, in order.
pub const RATE_STEPS: [f64; 4] = [20.0, 22.0, 24.0, 22.0];
/// A 4×4 work interval: rate kept down so strokes stay long (rowed at
/// 19–23 on the first one, 2026-11-22).
pub const FOUR_BY_FOUR_RATE: (f64, f64) = (20.0, 26.0);
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
            Self::FourByFour => "Norwegian 4×4: 4 hard intervals (heart rate 85–95% of max), 3 min easy between, building to 4 min each",
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

    /// The workout with its targets: `heart` gives the heart-rate ceilings,
    /// `progression` the power from last time (and an interval workout's
    /// length). A piece's length is the shortest the kind allows: the plan
    /// (or the command line) can set a longer one.
    pub fn workout(self, heart: Option<&HeartProfile>, progression: Option<&Progression>) -> Workout {
        let mut w = self.targets(heart);
        let band = |x: f64, k: f64| ((x * (1.0 - k)).round(), (x * (1.0 + k)).round());
        match (&mut w, progression) {
            (Workout::Intervals { work_s, work, .. }, Some(p)) => {
                *work_s = p.work_s.unwrap_or(*work_s);
                for (t, &x) in work.iter_mut().zip(&p.watts) {
                    t.watts = Some(band(x, POWER_BAND));
                }
            }
            (Workout::Piece { targets, .. }, Some(p)) => {
                // The pace's band: never narrower than PACE_BAND_MIN_W either side.
                let half = |x: f64| (x * PACE_BAND).max(PACE_BAND_MIN_W);
                targets.watts = p.watts.first().map(|&x| ((x - half(x)).round(), (x + half(x)).round()));
            }
            (_, None) => {}
        }
        w
    }

    fn targets(self, heart: Option<&HeartProfile>) -> Workout {
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

    /// Where the workout is `elapsed_s` after Go.
    pub fn at(&self, elapsed_s: f64) -> Position {
        let done = |of: u32| Position {
            stage: Stage::Done,
            n: of,
            of,
            left_s: None,
        };
        match self {
            Self::Piece { minutes, .. } => match minutes.map(|m| m * 60.0 - elapsed_s) {
                Some(left) if left <= 0.0 => done(1),
                left => Position {
                    stage: Stage::Work,
                    n: 1,
                    of: 1,
                    left_s: left,
                },
            },
            Self::Intervals { work_s, rest_s, work } => {
                let (w, r, of) = (*work_s as f64, *rest_s as f64, work.len() as u32);
                let k = (elapsed_s / (w + r)) as u32;
                let within = elapsed_s - k as f64 * (w + r);
                if k >= of || (k + 1 == of && within >= w) {
                    done(of)
                } else if within < w {
                    Position {
                        stage: Stage::Work,
                        n: k + 1,
                        of,
                        left_s: Some(w - within),
                    }
                } else {
                    Position {
                        stage: Stage::Rest,
                        n: k + 1,
                        of,
                        left_s: Some(w + r - within),
                    }
                }
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

/// Where a workout is: work interval `n` of `of` (a piece is 1 of 1), the
/// rest after it, or done; `left_s` of that part, when it has a length.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Position {
    pub stage: Stage,
    pub n: u32,
    pub of: u32,
    pub left_s: Option<f64>,
}

/// A work interval's power band: this much either side of its target.
pub const POWER_BAND: f64 = 0.04;
/// Held all the way: the next one goes up this much.
pub const STEP_UP: f64 = 0.03;
/// The last work interval within this much of the first: no fade.
pub const FADE: f64 = 0.03;
/// The 4×4's work intervals build to 4:00 through these lengths.
pub const FOUR_BY_FOUR_LENGTHS_S: [u32; 3] = [120, 180, 240];
/// Rated this hard (of 10) on the review screen: too hard, a step back.
pub const TOO_HARD_EFFORT: u8 = 9;

/// The next interval workout's power (and for the 4×4, the work intervals'
/// length), from how the last one of the same kind went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progression {
    /// For each work interval, in order.
    pub watts: Vec<f64>,
    /// The work intervals' length, when not the kind's own.
    pub work_s: Option<u32>,
    /// Why, in a line: what last time did and what that makes this time.
    pub note: String,
}

/// The next one of an interval workout, from the last one's work intervals
/// and how hard the rower rated it (`effort`, of 10).
///
/// The 4×4 builds its work intervals from 2:00 to 4:00 before its power
/// goes up. Too hard last time (not all four done, the last more than 3%
/// under the first, or rated 9 or 10): a step shorter, at its mean power.
/// Heart rate over the ceiling (`hr_top`): the same again. Otherwise held:
/// a step longer, and at 4:00, 3% more power.
///
/// Rate steps keep each step's own power, 3% more when every step was done
/// at its rate and not rated too hard. Unfinished, the steps done keep theirs.
///
/// `None` with nothing to go on: the first one is by heart rate (the 4×4)
/// or by rate alone (rate steps).
pub fn progression(kind: Kind, last: &[WorkInterval], hr_top: Option<f64>, effort: Option<u8>) -> Option<Progression> {
    let i = kind.intervals()?;
    let n = i.count as usize;
    let watts: Vec<f64> = last.iter().take(n).map_while(|w| w.seg.watts).collect();
    if watts.is_empty() {
        return None;
    }
    let finished = watts.len() == n;
    let rated_hard = effort.is_some_and(|e| e >= TOO_HARD_EFFORT);
    let (first, end) = (watts[0], watts[watts.len() - 1]);
    let w = |x: f64| format!("{x:.0} W");
    match kind {
        Kind::FourByFour => {
            let mean = watts.iter().sum::<f64>() / watts.len() as f64;
            let peak = last.iter().filter_map(|w| w.hr_peak).max_by(f64::total_cmp);
            // The length it was rowed at: the step nearest its intervals' time.
            let timed = last.iter().map(|w| w.length_s).fold(0.0, f64::max);
            let at = FOUR_BY_FOUR_LENGTHS_S
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| (**s as f64 - timed).abs() as u64)
                .map_or(FOUR_BY_FOUR_LENGTHS_S.len() - 1, |(k, _)| k);
            let longest = FOUR_BY_FOUR_LENGTHS_S.len() - 1;
            let too_hard = if !finished {
                Some(format!("{} of {n} done", watts.len()))
            } else if end < first * (1.0 - FADE) {
                Some(format!("faded from {} to {}", w(first), w(end)))
            } else if rated_hard {
                Some(format!("rated {}/10", effort.unwrap_or_default()))
            } else {
                None
            };
            let (step, power, why) = match (too_hard, hr_top.zip(peak)) {
                (Some(why), _) => (at.saturating_sub(1), mean, why),
                (None, Some((top, p))) if p > top => (at, mean, format!("heart rate reached {p:.0}, over {top:.0}")),
                (None, _) if at < longest => (at + 1, mean, "held all four".into()),
                (None, _) => (at, mean * (1.0 + STEP_UP), "held all four".into()),
            };
            let work_s = FOUR_BY_FOUR_LENGTHS_S[step];
            let power = power.round();
            Some(Progression {
                watts: vec![power; n],
                work_s: (work_s != i.work_s).then_some(work_s),
                note: format!(
                    "Last 4×4: {n} × {} at {} average, {why}: {n} × {} at {} this time.",
                    mmss(FOUR_BY_FOUR_LENGTHS_S[at]),
                    w(mean),
                    mmss(work_s),
                    w(power)
                ),
            })
        }
        Kind::RateSteps => {
            let on_rate = last.iter().zip(RATE_STEPS).all(|(l, r)| l.seg.spm.is_some_and(|s| (s - r).abs() <= 1.0));
            let k = if finished && on_rate && !rated_hard { 1.0 + STEP_UP } else { 1.0 };
            let next: Vec<f64> = watts.iter().map(|x| (x * k).round()).collect();
            let list = |v: &[f64]| v.iter().map(|x| format!("{x:.0}")).collect::<Vec<_>>().join(" / ");
            let why = match (finished, on_rate, rated_hard) {
                (false, _, _) => format!("{} of {n} steps done", watts.len()),
                (true, _, true) => format!("rated {}/10", effort.unwrap_or_default()),
                (true, true, false) => "every step at its rate".into(),
                (true, false, false) => "not every step at its rate".into(),
            };
            Some(Progression {
                note: format!("Last rate steps: {} W, {why}: {} W this time.", list(&watts), list(&next)),
                watts: next,
                work_s: None,
            })
        }
        _ => None,
    }
}

/// An easy or steady row's power, from how the last one ended (`last`):
/// the power of its last 10 minutes, a pace step lighter when heart rate
/// there was over the ceiling (`hr_top`), a step more when it was well
/// under, which is where the coach would have taken it next. `None` for a
/// row too short to have a last 10 minutes, or without power and heart rate
/// in them: the first one's power is set by heart rate as it's rowed.
pub fn easy_power(last: &Summary, hr_top: f64) -> Option<Progression> {
    let end = last.last_10_min.as_ref()?;
    let (w, hr) = (end.watts?, end.hr?);
    let (next, why) = if hr > hr_top {
        (cues::lighter(w).unwrap_or(w.round()), format!("over {hr_top:.0}"))
    } else if hr <= hr_top - cues::PACE_UNDER_BPM {
        (cues::heavier(w), format!("well under {hr_top:.0}"))
    } else {
        (w.round(), format!("close to {hr_top:.0}"))
    };
    Some(Progression {
        watts: vec![next],
        work_s: None,
        note: format!("Last easy row ended at {w:.0} W with heart rate {hr:.0}, {why}: {next:.0} W to start."),
    })
}

/// When each work interval of an interval workout ran: from the coach's
/// `Stage` events, or in a row recorded before those (timed by the PM5),
/// from the PM5's interval clock. A piece has none.
pub fn work_windows(events: &[Event]) -> Vec<(i64, i64)> {
    let stages: Vec<(i64, Stage, u32)> = events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::Stage { stage, n } => Some((e.t_us, stage, n)),
            _ => None,
        })
        .collect();
    if !stages.is_empty() {
        let intervals = stages.iter().any(|&(_, s, _)| s == Stage::Rest) || stages.iter().any(|&(_, s, n)| s == Stage::Work && n > 1);
        return if intervals { windows(stages.iter().map(|&(t, s, _)| (t, Some(s))), events) } else { Vec::new() };
    }
    let mut clock = IntervalClock::default();
    let polls = events.iter().filter_map(|e| match &e.kind {
        EventKind::Pm5Status(s) => Some((e.t_us, clock.poll(s))),
        _ => None,
    });
    windows(polls, events)
}

/// Work windows from stage changes in time order: each work to the next change.
fn windows(changes: impl Iterator<Item = (i64, Option<Stage>)>, events: &[Event]) -> Vec<(i64, i64)> {
    let mut windows = Vec::new();
    let mut open: Option<i64> = None;
    for (t_us, stage) in changes {
        match (stage, open) {
            (Some(Stage::Work), _) => open = Some(t_us),
            (Some(_), Some(from)) => {
                windows.push((from, t_us));
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


/// Whether the PM5 is running an interval workout: states 3–9 (work, rest
/// and the changes between, by Concept2's PM CSAFE spec).
fn in_intervals(code: u8) -> bool {
    (3..=9).contains(&code)
}

/// An interval clock jumping up by more than this has started the next interval.
const NEXT_INTERVAL_CS: u32 = 500;

/// Follows an interval workout timed by the PM5, for rows recorded before
/// the row screen timed them (the first 4×4, 2026-11-22: set up on the PM5
/// as six work intervals, the rests among them rowed easy, so the PM5 said 4,
/// interval work, throughout). What marks each interval is its clock
/// (`work_time_cs`), which counts it down and starts again at the next one's
/// length: the first interval is work, each reset moves to rest or back to
/// work, and leaving the interval states (10–12: ended, stopped, logged) is
/// the end.
#[derive(Debug, Clone, Default)]
pub struct IntervalClock {
    stage: Option<Stage>,
    left_cs: Option<u32>,
}

impl IntervalClock {
    pub fn stage(&self) -> Option<Stage> {
        self.stage
    }

    /// One status poll; the stage when it changes.
    pub fn poll(&mut self, s: &Pm5Status) -> Option<Stage> {
        let reset = matches!((self.left_cs, s.work_time_cs), (Some(was), Some(now)) if now > was + NEXT_INTERVAL_CS);
        self.left_cs = s.work_time_cs.or(self.left_cs);
        let running = in_intervals(s.workout_state?);
        let next = match self.stage {
            None if running => Stage::Work,
            Some(Stage::Work) if !running => Stage::Done,
            Some(Stage::Rest) if !running => Stage::Done,
            Some(Stage::Work) if reset => Stage::Rest,
            Some(Stage::Rest) if reset => Stage::Work,
            _ => return None,
        };
        self.stage = Some(next);
        Some(next)
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
        let Workout::Intervals { work_s, rest_s, work } = Kind::FourByFour.workout(Some(&h), None) else {
            panic!("intervals");
        };
        assert_eq!((work_s, rest_s, work.len()), (240, 180, 4));
        assert!(work.iter().all(|t| t.hr_max == Some(175.0) && t.rate == Some(FOUR_BY_FOUR_RATE)));
        assert!(Kind::FourByFour.is_hard());
    }

    #[test]
    fn rate_steps_step_the_rate() {
        let Workout::Intervals { work, .. } = Kind::RateSteps.workout(None, None) else {
            panic!("intervals");
        };
        let mids: Vec<f64> = work.iter().map(|t| t.rate.map(|(lo, hi)| (lo + hi) / 2.0).unwrap()).collect();
        assert_eq!(mids, RATE_STEPS);
    }

    #[test]
    fn an_easy_row_starts_where_the_last_one_would_have_gone_next() {
        let ended = |watts: f64, hr: f64| Summary {
            last_10_min: Some(crate::session::Segment {
                watts: Some(watts),
                hr: Some(hr),
                ..Default::default()
            }),
            ..Default::default()
        };
        let start = |s: &Summary| easy_power(s, 148.0).map(|p| p.watts[0]);
        assert_eq!(start(&ended(43.4, 151.0)), Some(40.0), "over: a step lighter");
        assert_eq!(start(&ended(40.0, 146.0)), Some(40.0), "close: the same");
        assert_eq!(start(&ended(40.0, 140.0)), Some(45.0), "well under: a step more");
        assert_eq!(start(&ended(28.0, 160.0)), Some(28.0), "never lighter than it was, at the floor");
        assert_eq!(start(&Summary::default()), None, "too short to say");
        assert_eq!(
            easy_power(&ended(43.4, 151.0), 148.0).unwrap().note,
            "Last easy row ended at 43 W with heart rate 151, over 148: 40 W to start."
        );
        let p = easy_power(&ended(40.0, 146.0), 148.0).unwrap();
        let Workout::Piece { targets, .. } = Kind::Easy.workout(None, Some(&p)) else { panic!() };
        assert_eq!(targets.watts, Some((35.0, 45.0)), "the pace's band, at least 5 W either side");
    }

    #[test]
    fn easy_ceiling_is_top_of_zone_2_and_the_plan_sets_its_length() {
        let h = HeartProfile::new(60.0, 190.0, None).unwrap();
        let mut easy = Kind::Easy.workout(Some(&h), None);
        easy.set_minutes(25.0);
        let Workout::Piece { minutes, targets } = easy else {
            panic!("a piece");
        };
        assert_eq!((minutes, targets.hr_max), (Some(25.0), Some(151.0)));
        let mut four = Kind::FourByFour.workout(None, None);
        four.set_minutes(40.0);
        assert_eq!(four, Kind::FourByFour.workout(None, None), "intervals keep their length");
    }

    /// Work intervals done at `length_s` each.
    fn done(watts: &[f64], length_s: f64, hr_peak: f64) -> Vec<WorkInterval> {
        watts
            .iter()
            .map(|&w| WorkInterval {
                seg: crate::session::Segment {
                    watts: Some(w),
                    spm: Some(22.0),
                    ..Default::default()
                },
                hr_peak: Some(hr_peak),
                length_s,
            })
            .collect()
    }

    fn four(last: &[WorkInterval], effort: Option<u8>) -> Progression {
        progression(Kind::FourByFour, last, Some(174.0), effort).unwrap()
    }

    #[test]
    fn the_four_by_four_builds_to_four_minutes_then_its_power_goes_up() {
        let p = four(&done(&[100.0, 101.0, 99.0, 100.0], 121.0, 168.0), None);
        assert_eq!((p.work_s, p.watts.clone()), (Some(180), vec![100.0; 4]), "held at 2:00: 3:00, same power");
        let p = four(&done(&[100.0; 4], 181.0, 168.0), None);
        assert_eq!((p.work_s, p.watts.clone()), (None, vec![100.0; 4]), "held at 3:00: 4:00");
        let p = four(&done(&[110.0, 108.0, 109.0, 109.0], 241.0, 170.0), Some(7));
        assert_eq!((p.work_s, p.watts.clone()), (None, vec![112.0; 4]), "held at 4:00: 3% more");
        assert_eq!(p.note, "Last 4×4: 4 × 4:00 at 109 W average, held all four: 4 × 4:00 at 112 W this time.");
    }

    #[test]
    fn a_four_by_four_too_hard_steps_back_and_over_the_ceiling_repeats() {
        // The first real one: three of four, fading, heart rate to 188.
        let first = four(&done(&[135.0, 91.0, 89.0], 241.0, 188.0), None);
        assert_eq!((first.work_s, first.watts.clone()), (Some(180), vec![105.0; 4]));
        assert_eq!(first.note, "Last 4×4: 4 × 4:00 at 105 W average, 3 of 4 done: 4 × 3:00 at 105 W this time.");
        let faded = four(&done(&[120.0, 112.0, 105.0, 100.0], 181.0, 170.0), None);
        assert_eq!((faded.work_s, faded.watts.clone()), (Some(120), vec![109.0; 4]));
        assert!(faded.note.contains("faded from 120 W to 100 W"), "{}", faded.note);
        let rated = four(&done(&[100.0; 4], 181.0, 168.0), Some(9));
        assert_eq!(rated.work_s, Some(120), "rated 9/10: a step back");
        assert!(rated.note.contains("rated 9/10"));
        let shortest = four(&done(&[100.0; 3], 121.0, 168.0), None);
        assert_eq!(shortest.work_s, Some(120), "never shorter than 2:00");
        let over = four(&done(&[110.0; 4], 241.0, 178.0), None);
        assert_eq!((over.work_s, over.watts.clone()), (None, vec![110.0; 4]));
        assert!(over.note.contains("over 174"));
        assert_eq!(progression(Kind::FourByFour, &[], Some(174.0), None), None);
        assert_eq!(progression(Kind::Steady, &done(&[110.0; 4], 241.0, 160.0), None, None), None);
    }

    #[test]
    fn rate_steps_keep_each_step_s_power_and_go_up_when_on_rate() {
        let mut last = done(&[80.0, 88.0, 95.0, 90.0], 300.0, 150.0);
        for (l, r) in last.iter_mut().zip(RATE_STEPS) {
            l.seg.spm = Some(r + 0.4);
        }
        let p = progression(Kind::RateSteps, &last, None, None).unwrap();
        assert_eq!(p.watts, vec![82.0, 91.0, 98.0, 93.0]);
        assert_eq!(p.work_s, None);
        assert_eq!(progression(Kind::RateSteps, &last, None, Some(10)).unwrap().watts, vec![80.0, 88.0, 95.0, 90.0]);
        last[2].seg.spm = Some(26.5);
        assert_eq!(progression(Kind::RateSteps, &last, None, None).unwrap().watts, vec![80.0, 88.0, 95.0, 90.0]);
    }

    #[test]
    fn the_progression_sets_the_work_intervals() {
        let p = Progression {
            watts: vec![110.0; 4],
            work_s: Some(180),
            note: String::new(),
        };
        let Workout::Intervals { work_s, work, .. } = Kind::FourByFour.workout(None, Some(&p)) else { panic!() };
        assert_eq!(work_s, 180);
        assert!(work.iter().all(|t| t.watts == Some((106.0, 114.0))));
    }

    /// A status poll: workout state and the interval clock (seconds left).
    fn poll(sec: i64, code: u8, left_s: u32) -> Event {
        Event {
            t_us: sec * 1_000_000,
            kind: EventKind::Pm5Status(model::Pm5Status {
                workout_state: Some(code),
                work_time_cs: Some(left_s * 100),
                ..Default::default()
            }),
        }
    }

    /// Polls a second apart as the real PM5 gave them on 2026-11-22: waiting
    /// (0), then 4 through work and rest alike with each interval's clock
    /// counting down, a one-poll 6 at one change, then ended and logged.
    fn real_pm5() -> Vec<Event> {
        let mut out = vec![poll(0, 0, 240)];
        let mut t = 1;
        for k in 0..3 {
            for left in (1..=240).rev() {
                out.push(poll(t, if k == 2 && left == 240 { 6 } else { 4 }, left));
                t += 1;
            }
            for left in (1..=180).rev() {
                out.push(poll(t, 4, left));
                t += 1;
            }
        }
        out.extend([poll(t, 10, 0), poll(t + 1, 12, 0), poll(t + 2, 13, 0), poll(t + 3, 0, 0)]);
        out
    }

    #[test]
    fn the_interval_clock_finds_work_and_rest_on_a_real_pm5() {
        let mut clock = IntervalClock::default();
        let changes: Vec<(i64, Stage)> = real_pm5()
            .iter()
            .filter_map(|e| match &e.kind {
                EventKind::Pm5Status(s) => clock.poll(s).map(|st| (e.t_us / 1_000_000, st)),
                _ => None,
            })
            .collect();
        use Stage::*;
        assert_eq!(
            changes,
            [(1, Work), (241, Rest), (421, Work), (661, Rest), (841, Work), (1081, Rest), (1261, Done)]
        );
        let mut piece = IntervalClock::default();
        let EventKind::Pm5Status(s) = poll(5, 1, 300).kind else { panic!() };
        assert_eq!(piece.poll(&s), None, "a single piece isn't intervals");
    }

    #[test]
    fn where_a_workout_is() {
        let four = Kind::FourByFour.workout(None, None);
        let at = |s: f64| {
            let p = four.at(s);
            (p.stage, p.n, p.left_s.map(|l| l.round()))
        };
        assert_eq!(at(0.0), (Stage::Work, 1, Some(240.0)));
        assert_eq!(at(250.0), (Stage::Rest, 1, Some(170.0)));
        assert_eq!(at(420.0), (Stage::Work, 2, Some(240.0)));
        assert_eq!(at(3.0 * 420.0 + 239.0), (Stage::Work, 4, Some(1.0)));
        assert_eq!(at(3.0 * 420.0 + 240.0), (Stage::Done, 4, None), "no rest after the last");
        let piece = Kind::Reference.workout(None, None);
        assert_eq!((piece.at(60.0).stage, piece.at(60.0).left_s), (Stage::Work, Some(1140.0)));
        assert_eq!(piece.at(1200.0).stage, Stage::Done);
    }

    #[test]
    fn work_windows_come_from_the_coach_s_stages() {
        let stage = |sec: i64, stage: Stage, n: u32| Event {
            t_us: sec * 1_000_000,
            kind: EventKind::Stage { stage, n },
        };
        let events = [stage(10, Stage::Work, 1), stage(250, Stage::Rest, 1), stage(430, Stage::Work, 2), stage(670, Stage::Done, 2)];
        assert_eq!(work_windows(&events), vec![(10_000_000, 250_000_000), (430_000_000, 670_000_000)]);
        let piece = [stage(10, Stage::Work, 1), stage(1210, Stage::Done, 1)];
        assert!(work_windows(&piece).is_empty(), "a piece has no work intervals");
    }

    #[test]
    fn work_windows_follow_the_pm5() {
        let w = work_windows(&real_pm5());
        assert_eq!(
            w,
            vec![(1_000_000, 241_000_000), (421_000_000, 661_000_000), (841_000_000, 1_081_000_000)]
        );
        assert!(work_windows(&[poll(0, 1, 0), poll(5, 1, 5)]).is_empty(), "a single piece has none");
    }

}
