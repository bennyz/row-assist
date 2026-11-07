//! Pacing cues: short spoken corrections when rate, power or heart rate
//! drift off target, plus halfway and time calls.
//!
//! Judged on the median of the last few strokes so one odd stroke says
//! nothing, with a settling-in period and cooldowns so it never nags. The
//! PM5 shows the exact numbers; cues only say which way to go.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::strokes::StrokeRecord;
use crate::technique::Fault;

/// Acceptable ranges. Each is `(low, high)`, inclusive; a cue fires outside it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Targets {
    pub rate: Option<(f64, f64)>,
    pub watts: Option<(f64, f64)>,
    /// Heart-rate ceiling, bpm.
    pub hr_max: Option<f64>,
    /// Piece length; enables the halfway and time calls.
    pub minutes: Option<f64>,
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
        }
    }

    fn kind(&self) -> usize {
        match self {
            Self::HeartHigh { .. } => 0,
            Self::RateUp { .. } | Self::RateDown { .. } => 1,
            Self::Harder { .. } | Self::Easier { .. } => 2,
            Self::Halfway | Self::Time => 3,
        }
    }
}

/// Strokes the median is taken over.
pub const WINDOW: usize = 6;
/// No corrections in the first part of a piece, while rate and HR settle.
pub const SETTLE_S: f64 = 60.0;
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
    targets: Targets,
    recent: VecDeque<Recent>,
    first_catch_us: Option<i64>,
    last_any_us: Option<i64>,
    last_kind_us: [Option<i64>; 4],
    halfway_said: bool,
    time_said: bool,
}

impl Pacer {
    pub fn new(targets: Targets) -> Self {
        Self {
            targets,
            recent: VecDeque::with_capacity(WINDOW),
            first_catch_us: None,
            last_any_us: None,
            last_kind_us: [None; 4],
            halfway_said: false,
            time_said: false,
        }
    }

    pub fn targets(&self) -> &Targets {
        &self.targets
    }

    /// Feed each completed stroke; returns at most one cue.
    pub fn stroke(&mut self, s: &StrokeRecord) -> Option<Cue> {
        let now = s.t_next_catch_us.unwrap_or(s.t_catch_us);
        let start = *self.first_catch_us.get_or_insert(s.t_catch_us);
        let elapsed_s = (now - start) as f64 / 1e6;
        if self.recent.len() == WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(Recent {
            rate: s.rate().or(s.spm.map(f64::from)),
            watts: s.watts.map(f64::from),
            hr: s.hr_bpm,
        });

        // Time calls always get through.
        if let Some(min) = self.targets.minutes {
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
        if self.time_said || elapsed_s < SETTLE_S || self.recent.len() < WINDOW {
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
        let t = &self.targets;
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

    fn run(targets: Targets, strokes: &[StrokeRecord]) -> Vec<(f64, Cue)> {
        let mut p = Pacer::new(targets);
        strokes
            .iter()
            .filter_map(|s| p.stroke(s).map(|c| ((s.t_catch_us - T0) as f64 / 1e6, c)))
            .collect()
    }

    fn reference() -> Targets {
        Targets {
            rate: Some((19.0, 21.0)),
            watts: Some((48.0, 58.0)),
            hr_max: None,
            minutes: Some(20.0),
        }
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
        let cues = run(t.clone(), &steady(60, 3.0, 40.0, |_| 160.0));
        assert!(matches!(cues[0].1, Cue::HeartHigh { bpm } if bpm == 160.0));
        assert!(cues.iter().all(|(_, c)| matches!(c, Cue::HeartHigh { .. })));
        // With heart rate fine, power gets its turn.
        let cues = run(t, &steady(60, 3.0, 40.0, |_| 140.0));
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
        assert!(run(t, &s).is_empty());
    }

    #[test]
    fn texts() {
        assert_eq!(Cue::RateUp { target: 20.0 }.text(), "Rate up. 20.");
        assert_eq!(Cue::HeartHigh { bpm: 162.4 }.text(), "Heart rate 162. Ease off.");
        assert!(Targets::default().is_empty());
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
