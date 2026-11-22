//! Pacing cues: short spoken corrections when rate, power or heart rate
//! drift off target, plus halfway and time calls; for intervals, a call as
//! each work interval and rest begins.
//!
//! Judged on the median of the last few strokes so one odd stroke says
//! nothing, with a settling-in period and cooldowns so it never nags. The
//! PM5 shows the exact numbers; cues only say which way to go.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::strokes::StrokeRecord;
use crate::technique::Fault;
use crate::workout::{IntervalClock, Stage, Workout};
use model::Pm5Status;

/// Acceptable ranges. Each is `(low, high)`, inclusive; a cue fires outside it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Targets {
    pub rate: Option<(f64, f64)>,
    pub watts: Option<(f64, f64)>,
    /// Heart-rate ceiling, bpm.
    pub hr_max: Option<f64>,
}

impl Targets {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The stroke rhythm at the middle of the target rate, if there is one.
    pub fn rhythm(&self) -> Option<Rhythm> {
        self.rate.map(|(lo, hi)| Rhythm::at((lo + hi) / 2.0))
    }
}

/// How long a stroke's drive and recovery should take, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rhythm {
    pub rate: f64,
    pub drive_s: f64,
    pub recovery_s: f64,
}

impl Rhythm {
    /// A third of the stroke on the drive (1 s of 3 at rate 20: recovery
    /// twice the drive), never quicker than 0.8 s or slower than 1.1 s.
    pub fn at(rate: f64) -> Self {
        let cycle = 60.0 / rate;
        let drive_s = (cycle / 3.0).clamp(0.8, 1.1);
        Self {
            rate,
            drive_s,
            recovery_s: cycle - drive_s,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cue", rename_all = "snake_case")]
pub enum Cue {
    HeartHigh { bpm: f64 },
    RateUp { target: f64 },
    RateDown { target: f64 },
    Harder { target: f64 },
    Easier { target: f64 },
    Halfway,
    Time,
    /// A work interval begins: the `n`th of `of`, with its rate and power
    /// when it has them.
    Go { n: u32, of: u32, rate: Option<f64>, watts: Option<f64> },
    Rest,
}

impl Cue {
    /// What to say.
    pub fn text(&self) -> String {
        match *self {
            Self::HeartHigh { bpm } => format!("Heart rate {bpm:.0}. Ease off."),
            Self::RateUp { target } => format!("Rate up. {target:.0}."),
            Self::RateDown { target } => format!("Rate down. {target:.0}."),
            Self::Harder { target } => format!("A bit harder. {target:.0} watts."),
            Self::Easier { target } => format!("Ease off. {target:.0} watts."),
            Self::Halfway => "Halfway.".into(),
            Self::Time => "Time. Paddle down.".into(),
            Self::Go { n, of, rate, watts } => {
                let mut s = if n == of && of > 1 { "Last one.".to_string() } else { format!("Go. {n} of {of}.") };
                if let Some(r) = rate {
                    s += &format!(" Rate {r:.0}.");
                }
                if let Some(w) = watts {
                    s += &format!(" {w:.0} watts.");
                }
                s
            }
            Self::Rest => "Rest. Paddle easy.".into(),
        }
    }

    fn kind(&self) -> usize {
        match self {
            Self::HeartHigh { .. } => 0,
            Self::RateUp { .. } | Self::RateDown { .. } => 1,
            Self::Harder { .. } | Self::Easier { .. } => 2,
            Self::Halfway | Self::Time | Self::Go { .. } | Self::Rest => 3,
        }
    }
}

/// Strokes the median is taken over.
pub const WINDOW: usize = 6;
/// No corrections in the first part of a piece, while rate and HR settle.
pub const SETTLE_S: f64 = 60.0;
/// Nor in the first part of a work interval, while rate and power come up.
pub const WORK_SETTLE_S: f64 = 30.0;
/// Quiet time after any correction.
pub const COOLDOWN_S: f64 = 30.0;
/// The same correction isn't repeated sooner than this.
pub const REPEAT_S: f64 = 60.0;

/// What the pacer keeps of each recent stroke.
#[derive(Debug, Clone, Copy)]
struct Recent {
    rate: Option<f64>,
    watts: Option<f64>,
    hr: Option<f64>,
}

#[derive(Debug)]
pub struct Pacer {
    workout: Workout,
    recent: VecDeque<Recent>,
    first_catch_us: Option<i64>,
    last_any_us: Option<i64>,
    last_kind_us: [Option<i64>; 4],
    halfway_said: bool,
    /// The end was called: the piece's time, or the last work interval's.
    time_said: bool,
    /// Intervals: where the PM5 is, and since when.
    clock: IntervalClock,
    since_us: i64,
    /// Intervals: work intervals begun.
    begun: u32,
}

/// Where an interval workout is, for the screens.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct IntervalNow {
    pub stage: Stage,
    /// The work interval this is, or the one before this rest.
    pub n: u32,
    pub of: u32,
    /// Seconds left by the plan (the PM5 shows the exact time).
    pub left_s: f64,
}

impl Pacer {
    pub fn new(workout: Workout) -> Self {
        Self {
            workout,
            recent: VecDeque::with_capacity(WINDOW),
            first_catch_us: None,
            last_any_us: None,
            last_kind_us: [None; 4],
            halfway_said: false,
            time_said: false,
            clock: IntervalClock::default(),
            since_us: 0,
            begun: 0,
        }
    }

    pub fn workout(&self) -> &Workout {
        &self.workout
    }

    /// What the row aims for right now: the piece's targets, or the
    /// current work interval's (none in a rest).
    pub fn targets(&self) -> Option<&Targets> {
        match &self.workout {
            Workout::Piece { targets, .. } => Some(targets),
            Workout::Intervals { work, .. } => match self.clock.stage() {
                Some(Stage::Work) if !self.time_said => work.get(self.begun as usize - 1),
                _ => None,
            },
        }
    }

    pub fn interval(&self, now_us: i64) -> Option<IntervalNow> {
        let Workout::Intervals { work_s, rest_s, work } = &self.workout else {
            return None;
        };
        let (stage, since) = (self.clock.stage()?, self.since_us);
        let length = match stage {
            Stage::Work => *work_s,
            Stage::Rest => *rest_s,
            Stage::Done => 0,
        };
        Some(IntervalNow {
            stage,
            n: self.begun,
            of: work.len() as u32,
            left_s: (length as f64 - (now_us - since) as f64 / 1e6).max(0.0),
        })
    }

    /// Every PM5 status poll. For intervals, a call when work or rest begins
    /// (`IntervalClock`), and the time call when the last work interval ends
    /// or the PM5's workout does. A piece ignores it.
    pub fn pm5(&mut self, s: &Pm5Status, t_us: i64) -> Option<Cue> {
        let Workout::Intervals { work, .. } = &self.workout else {
            return None;
        };
        let of = work.len() as u32;
        let now = self.clock.poll(s)?;
        if self.time_said {
            return None;
        }
        self.since_us = t_us;
        // A median across work and rest would say nothing about either.
        self.recent.clear();
        let cue = match now {
            Stage::Work => {
                self.begun += 1;
                let t = work.get(self.begun as usize - 1)?;
                let mid = |b: Option<(f64, f64)>| b.map(|(lo, hi)| (lo + hi) / 2.0);
                Cue::Go {
                    n: self.begun,
                    of,
                    rate: mid(t.rate),
                    watts: mid(t.watts),
                }
            }
            Stage::Rest if self.begun < of => Cue::Rest,
            Stage::Rest | Stage::Done => {
                self.time_said = true;
                Cue::Time
            }
        };
        Some(self.said(cue, t_us))
    }

    /// Feed each completed stroke; returns at most one cue.
    pub fn stroke(&mut self, s: &StrokeRecord) -> Option<Cue> {
        let now = s.t_next_catch_us.unwrap_or(s.t_catch_us);
        let start = *self.first_catch_us.get_or_insert(s.t_catch_us);
        if self.recent.len() == WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(Recent {
            rate: s.rate().or(s.spm.map(f64::from)),
            watts: s.watts.map(f64::from),
            hr: s.hr_bpm,
        });

        let settled = match &self.workout {
            Workout::Piece { minutes, .. } => {
                let elapsed_s = (now - start) as f64 / 1e6;
                // Time calls always get through.
                if let Some(min) = *minutes {
                    if !self.time_said && elapsed_s >= min * 60.0 {
                        self.time_said = true;
                        self.halfway_said = true;
                        return Some(self.said(Cue::Time, now));
                    }
                    if !self.halfway_said && elapsed_s >= min * 30.0 {
                        self.halfway_said = true;
                        return Some(self.said(Cue::Halfway, now));
                    }
                }
                elapsed_s >= SETTLE_S
            }
            Workout::Intervals { .. } => {
                self.clock.stage() == Some(Stage::Work) && (now - self.since_us) as f64 / 1e6 >= WORK_SETTLE_S
            }
        };
        if self.time_said || !settled || self.recent.len() < WINDOW {
            return None;
        }
        if self.last_any_us.is_some_and(|t| (now - t) as f64 / 1e6 < COOLDOWN_S) {
            return None;
        }

        let cue = self.correction()?;
        let repeat_ok = self.last_kind_us[cue.kind()].is_none_or(|t| (now - t) as f64 / 1e6 >= REPEAT_S);
        repeat_ok.then(|| self.said(cue, now))
    }

    /// Most important correction first: heart rate, then rate, then power.
    fn correction(&self) -> Option<Cue> {
        let median = |pick: fn(&Recent) -> Option<f64>| {
            let mut xs: Vec<f64> = self.recent.iter().filter_map(pick).collect();
            if xs.len() < WINDOW / 2 {
                return None;
            }
            xs.sort_by(f64::total_cmp);
            Some(xs[xs.len() / 2])
        };
        let t = self.targets()?;
        if let (Some(max), Some(hr)) = (t.hr_max, median(|r| r.hr))
            && hr > max
        {
            return Some(Cue::HeartHigh { bpm: hr });
        }
        if let (Some((lo, hi)), Some(rate)) = (t.rate, median(|r| r.rate)) {
            let target = (lo + hi) / 2.0;
            if rate < lo {
                return Some(Cue::RateUp { target });
            }
            if rate > hi {
                return Some(Cue::RateDown { target });
            }
        }
        if let (Some((lo, hi)), Some(w)) = (t.watts, median(|r| r.watts)) {
            let target = (lo + hi) / 2.0;
            if w < lo {
                return Some(Cue::Harder { target });
            }
            if w > hi {
                return Some(Cue::Easier { target });
            }
        }
        None
    }

    fn said(&mut self, cue: Cue, now: i64) -> Cue {
        self.last_any_us = Some(now);
        self.last_kind_us[cue.kind()] = Some(now);
        cue
    }
}

/// Technique coaching with one focus per session (DESIGN.md §5.2): speak
/// when the focus fault shows in most recent strokes, then leave the rower
/// alone to fix it; praise once it has stayed fixed.
#[derive(Debug)]
pub struct FocusCoach {
    focus: Fault,
    /// Last judged strokes: did the focus fault show?
    recent: VecDeque<bool>,
    last_cue_us: Option<i64>,
    /// A correction was given and not yet followed by praise.
    corrected: bool,
    good_streak: u32,
}

/// Strokes looked back over, and how many must show the fault.
pub const FOCUS_WINDOW: usize = 10;
pub const FOCUS_HITS: usize = 6;
/// Quiet time after a correction.
pub const FOCUS_COOLDOWN_S: f64 = 30.0;
/// Praise after this many strokes with the fault in under 2 of the last 10.
pub const GOOD_STREAK: u32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cue", rename_all = "snake_case")]
pub enum TechniqueCue {
    Fix { fault: Fault },
    Good,
}

impl TechniqueCue {
    pub fn text(&self) -> &'static str {
        match self {
            Self::Fix { fault } => fault.cue(),
            Self::Good => "That's it.",
        }
    }
}

impl FocusCoach {
    pub fn new(focus: Fault) -> Self {
        Self {
            focus,
            recent: VecDeque::with_capacity(FOCUS_WINDOW),
            last_cue_us: None,
            corrected: false,
            good_streak: 0,
        }
    }

    pub fn focus(&self) -> Fault {
        self.focus
    }

    /// Feed each completed stroke (with its faults attached).
    pub fn stroke(&mut self, s: &StrokeRecord) -> Option<TechniqueCue> {
        // A stroke the camera didn't see can't show a camera fault: skip it.
        if s.t_next_catch_us.is_none() || (self.focus.needs_camera() && s.pose.is_none()) {
            return None;
        }
        let now = s.t_next_catch_us.unwrap_or(s.t_catch_us);
        if self.recent.len() == FOCUS_WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(s.faults.iter().any(|h| h.fault == self.focus));
        let hits = self.recent.iter().filter(|&&h| h).count();

        let quiet = self.last_cue_us.is_none_or(|t| (now - t) as f64 / 1e6 >= FOCUS_COOLDOWN_S);
        if self.recent.len() == FOCUS_WINDOW && hits >= FOCUS_HITS && quiet {
            self.last_cue_us = Some(now);
            self.corrected = true;
            self.good_streak = 0;
            return Some(TechniqueCue::Fix { fault: self.focus });
        }
        if self.corrected {
            self.good_streak = if hits < 2 { self.good_streak + 1 } else { 0 };
            if self.good_streak >= GOOD_STREAK {
                self.corrected = false;
                self.good_streak = 0;
                self.last_cue_us = Some(now);
                return Some(TechniqueCue::Good);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tests::{T0, steady};

    #[test]
    fn the_rhythm_puts_a_third_of_the_stroke_on_the_drive() {
        let at = |rate: f64| {
            let r = Rhythm::at(rate);
            ((r.drive_s * 100.0).round() / 100.0, (r.recovery_s * 100.0).round() / 100.0)
        };
        assert_eq!(at(20.0), (1.0, 2.0));
        assert_eq!(at(18.0), (1.1, 2.23), "a long stroke: the drive doesn't drag on");
        assert_eq!(at(30.0), (0.8, 1.2), "a quick one: the drive can't be rushed");
        let reference = Targets {
            rate: Some((19.0, 21.0)),
            ..Default::default()
        };
        assert_eq!(reference.rhythm().map(|r| r.rate), Some(20.0));
        assert_eq!(Targets::default().rhythm(), None);
    }

    fn piece(minutes: Option<f64>, targets: Targets) -> Workout {
        Workout::Piece { minutes, targets }
    }

    fn run(workout: Workout, strokes: &[StrokeRecord]) -> Vec<(f64, Cue)> {
        let mut p = Pacer::new(workout);
        strokes
            .iter()
            .filter_map(|s| p.stroke(s).map(|c| ((s.t_catch_us - T0) as f64 / 1e6, c)))
            .collect()
    }

    fn reference() -> Workout {
        piece(
            Some(20.0),
            Targets {
                rate: Some((19.0, 21.0)),
                watts: Some((48.0, 58.0)),
                hr_max: None,
            },
        )
    }

    #[test]
    fn on_target_only_hears_the_time_calls() {
        let cues = run(reference(), &steady(420, 3.0, 53.0, |_| 140.0));
        let kinds: Vec<_> = cues.iter().map(|(_, c)| *c).collect();
        assert_eq!(kinds, vec![Cue::Halfway, Cue::Time]);
        assert!((cues[0].0 - 597.0).abs() < 0.1, "halfway at {}", cues[0].0);
    }

    #[test]
    fn rating_high_is_corrected_after_settling_with_cooldowns() {
        // 24 spm the whole way: too fast.
        let cues = run(reference(), &steady(120, 2.5, 53.0, |_| 140.0));
        let first = cues[0];
        assert!(matches!(first.1, Cue::RateDown { target } if target == 20.0));
        assert!(first.0 >= SETTLE_S - 2.5, "no cue while settling: {}", first.0);
        // Same correction no more than once a minute.
        for pair in cues.windows(2) {
            assert!(pair[1].0 - pair[0].0 >= REPEAT_S - 2.5);
        }
        assert!(cues.len() >= 3);
    }

    #[test]
    fn heart_rate_outranks_power() {
        let t = Targets {
            watts: Some((48.0, 58.0)),
            hr_max: Some(150.0),
            ..Default::default()
        };
        // Power is low too, but "harder" with heart rate over the ceiling
        // would be the wrong advice: only the heart cue is heard.
        let cues = run(piece(None, t.clone()), &steady(60, 3.0, 40.0, |_| 160.0));
        assert!(matches!(cues[0].1, Cue::HeartHigh { bpm } if bpm == 160.0));
        assert!(cues.iter().all(|(_, c)| matches!(c, Cue::HeartHigh { .. })));
        // With heart rate fine, power gets its turn.
        let cues = run(piece(None, t), &steady(60, 3.0, 40.0, |_| 140.0));
        assert!(matches!(cues[0].1, Cue::Harder { target } if target == 53.0));
    }

    #[test]
    fn one_bad_stroke_says_nothing() {
        let mut s = steady(60, 3.0, 53.0, |_| 140.0);
        s[40].watts = Some(90);
        let t = Targets {
            watts: Some((48.0, 58.0)),
            ..Default::default()
        };
        assert!(run(piece(None, t), &s).is_empty());
    }

    #[test]
    fn texts() {
        assert_eq!(Cue::RateUp { target: 20.0 }.text(), "Rate up. 20.");
        assert_eq!(Cue::HeartHigh { bpm: 162.4 }.text(), "Heart rate 162. Ease off.");
        assert!(Targets::default().is_empty());
        let go = |n, rate, watts| Cue::Go { n, of: 4, rate, watts }.text();
        assert_eq!(go(1, None, None), "Go. 1 of 4.");
        assert_eq!(go(2, Some(22.0), Some(110.0)), "Go. 2 of 4. Rate 22. 110 watts.");
        assert_eq!(go(4, None, None), "Last one.");
    }

    mod intervals {
        use super::super::*;
        use crate::session::tests::{T0, steady};
        use crate::workout::Kind;

        const S: i64 = 1_000_000;

        /// A 4×4 (2 work intervals of 4 min, 3 min rest) as the PM5 times
        /// it, rowed at `spm` and `watts` throughout: each poll's state and
        /// each stroke, in time order, through the pacer.
        fn row(work: Vec<Targets>, spm: f64, watts: f64, hr: f64) -> (Pacer, Vec<(f64, Cue)>) {
            let mut p = Pacer::new(Workout::Intervals { work_s: 240, rest_s: 180, work });
            let n = (60.0 * 17.0 / (60.0 / spm)) as u32;
            let strokes = steady(n, 60.0 / spm, watts, |_| hr);
            // Work 0–240 s, rest to 420, work to 660, rest after; a poll a
            // second, as a real PM5 gives them: state 4 throughout, each
            // interval's clock counting down.
            let left_s = |sec: i64| match sec {
                0..240 => 240 - sec,
                240..420 => 420 - sec,
                420..660 => 660 - sec,
                _ => (840 - sec).max(1),
            };
            let mut cues = Vec::new();
            let mut next = strokes.iter().peekable();
            for sec in 0..17 * 60 {
                let t = T0 + sec * S;
                let s = Pm5Status {
                    workout_state: Some(4),
                    work_time_cs: Some(left_s(sec) as u32 * 100),
                    ..Default::default()
                };
                if let Some(c) = p.pm5(&s, t) {
                    cues.push((sec as f64, c));
                }
                while let Some(s) = next.next_if(|s| s.t_next_catch_us.unwrap_or(s.t_catch_us) <= t) {
                    // Judged as the stroke ends.
                    if let Some(c) = p.stroke(s) {
                        cues.push(((s.t_next_catch_us.unwrap() - T0) as f64 / 1e6, c));
                    }
                }
            }
            (p, cues)
        }

        fn hard(n: usize) -> Vec<Targets> {
            vec![
                Targets {
                    rate: Some((24.0, 28.0)),
                    watts: None,
                    hr_max: Some(175.0),
                };
                n
            ]
        }

        #[test]
        fn calls_each_interval_from_the_pm5_and_ends_after_the_last() {
            let (p, cues) = row(hard(2), 26.0, 110.0, 160.0);
            let calls: Vec<(f64, Cue)> = cues.into_iter().filter(|(_, c)| c.kind() == 3).collect();
            let go = |n| Cue::Go { n, of: 2, rate: Some(26.0), watts: None };
            assert_eq!(calls, vec![(0.0, go(1)), (240.0, Cue::Rest), (420.0, go(2)), (660.0, Cue::Time)]);
            assert!(p.targets().is_none(), "nothing to aim for once it's over");
        }

        #[test]
        fn corrects_in_work_only_after_settling_and_never_in_a_rest() {
            // Rate 31 throughout: too quick in work, and nobody cares in a rest.
            let (_, cues) = row(hard(2), 31.0, 110.0, 160.0);
            let fixes: Vec<f64> = cues.iter().filter(|(_, c)| matches!(c, Cue::RateDown { .. })).map(|(t, _)| *t).collect();
            assert!(!fixes.is_empty());
            for t in &fixes {
                let in_work = (WORK_SETTLE_S..240.0).contains(t) || (420.0 + WORK_SETTLE_S..660.0).contains(t);
                assert!(in_work, "a correction at {t} s");
            }
        }

        #[test]
        fn the_heart_ceiling_eases_a_bout_off() {
            let (_, cues) = row(hard(2), 26.0, 110.0, 180.0);
            assert!(cues.iter().any(|(_, c)| matches!(c, Cue::HeartHigh { bpm } if *bpm == 180.0)));
        }

        fn status(code: u8, left_s: u32) -> Pm5Status {
            Pm5Status {
                workout_state: Some(code),
                work_time_cs: Some(left_s * 100),
                ..Default::default()
            }
        }

        #[test]
        fn waiting_and_a_piece_ignore_the_pm5() {
            let mut p = Pacer::new(Kind::FourByFour.workout(None, None));
            assert_eq!(p.pm5(&status(0, 240), T0), None, "waiting for the first stroke");
            assert_eq!(p.pm5(&status(1, 0), T0), None, "a single piece on the PM5");
            assert!(matches!(p.pm5(&status(4, 240), T0 + S), Some(Cue::Go { n: 1, of: 4, .. })));
            assert_eq!(p.pm5(&status(4, 239), T0 + 2 * S), None, "the same interval");
            let now = p.interval(T0 + 61 * S).unwrap();
            assert_eq!((now.stage, now.n, now.of, now.left_s), (Stage::Work, 1, 4, 180.0));
            let mut piece = Pacer::new(Kind::Steady.workout(None, None));
            assert_eq!(piece.pm5(&status(4, 240), T0), None);
            assert!(piece.interval(T0).is_none());
        }

        #[test]
        fn stopping_early_on_the_pm5_ends_it() {
            let mut p = Pacer::new(Kind::FourByFour.workout(None, None));
            p.pm5(&status(4, 240), T0);
            p.pm5(&status(4, 1), T0 + 239 * S);
            assert_eq!(p.pm5(&status(4, 180), T0 + 240 * S), Some(Cue::Rest), "the clock starts again: a rest");
            assert_eq!(p.pm5(&status(12, 0), T0 + 300 * S), Some(Cue::Time), "logged after one");
            assert!(p.targets().is_none());
        }
    }

    mod focus {
        use super::super::*;
        use crate::session::tests::steady;
        use crate::technique::{FaultHit, StrokePose};

        /// Strokes with the fault on the ones `bad` picks.
        fn rowed(n: u32, bad: impl Fn(u32) -> bool) -> Vec<StrokeRecord> {
            let mut s = steady(n, 3.0, 53.0, |_| 140.0);
            for r in &mut s {
                r.pose = Some(StrokePose::default());
                if bad(r.index) {
                    r.faults = vec![FaultHit { fault: Fault::EarlyArms, value: 140.0 }];
                }
            }
            s
        }

        fn cues(strokes: &[StrokeRecord]) -> Vec<(u32, TechniqueCue)> {
            let mut c = FocusCoach::new(Fault::EarlyArms);
            strokes.iter().filter_map(|s| c.stroke(s).map(|q| (s.index, q))).collect()
        }

        #[test]
        fn quiet_on_clean_strokes_and_on_the_odd_fault() {
            assert!(cues(&rowed(200, |_| false)).is_empty());
            assert!(cues(&rowed(200, |i| i % 3 == 0)).is_empty(), "4 in 10 is not enough");
        }

        #[test]
        fn corrects_then_waits_then_praises_the_fix() {
            // Fault on strokes 0–59, fixed from 60 on.
            let c = cues(&rowed(150, |i| i < 60));
            let fix = TechniqueCue::Fix { fault: Fault::EarlyArms };
            assert_eq!(c[0], (9, fix), "once 6 of the first 10 show it");
            // 30 s (10 strokes at 3 s) between corrections.
            let fixes: Vec<u32> = c.iter().filter(|(_, q)| *q == fix).map(|(i, _)| *i).collect();
            assert!(fixes.windows(2).all(|w| w[1] - w[0] >= 10), "{fixes:?}");
            // Praise once the fix has held: under 2 in 10 for 20 strokes.
            let (praised_at, good) = *c.last().unwrap();
            assert_eq!(good, TechniqueCue::Good);
            assert!((80..=95).contains(&praised_at), "praised at {praised_at}");
            assert_eq!(c.iter().filter(|(_, q)| *q == TechniqueCue::Good).count(), 1);
            assert_eq!(fix.text(), "Legs first, arms long");
        }

        #[test]
        fn strokes_the_camera_missed_dont_count() {
            let mut s = rowed(40, |_| true);
            for r in &mut s {
                r.pose = None;
            }
            assert!(cues(&s).is_empty());
        }
    }
}
