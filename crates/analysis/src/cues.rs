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
use crate::workout::{Position, Workout};
use model::Stage;

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
    /// The next work interval is close.
    Soon,
    /// Heart rate (`bpm`) has stayed over the ceiling: a step lighter and a
    /// stroke slower. The new power and rate, each when it changed.
    Pace { bpm: f64, watts: Option<f64>, rate: Option<f64> },
    /// The pace is as light as it goes and heart rate is still over the
    /// ceiling: hold it there. Said once.
    Held { bpm: f64 },
}

impl Cue {
    /// What to say.
    pub fn text(&self) -> String {
        match *self {
            Self::RateUp { target } => format!("Rate up. {target:.0}."),
            Self::RateDown { target } => format!("Rate down. {target:.0}."),
            Self::Harder { target } => format!("A bit harder. {target:.0} watts."),
            Self::Easier { target } => format!("Ease off. {target:.0} watts."),
            Self::Halfway => "Halfway.".into(),
            Self::Time => "Time. Paddle down.".into(),
            Self::Go { n, of, rate, watts } => {
                let mut s = match (n, of) {
                    (_, 1) => "Go.".to_string(),
                    _ if n == of => "Last one.".to_string(),
                    _ => format!("Go. {n} of {of}."),
                };
                if let Some(r) = rate {
                    s += &format!(" Rate {r:.0}.");
                }
                if let Some(w) = watts {
                    s += &format!(" {w:.0} watts.");
                }
                s
            }
            Self::Rest => "Rest. Paddle easy.".into(),
            Self::Soon => "Ten seconds.".into(),
            Self::Pace { bpm, watts, rate } => {
                let to: Vec<String> = watts
                    .map(|w| format!("{w:.0} watts"))
                    .into_iter()
                    .chain(rate.map(|r| format!("rate {r:.0}")))
                    .collect();
                let way = if watts.is_some() { "Lighter" } else { "Slower" };
                format!("Heart rate {bpm:.0}. {way}: {}.", to.join(", "))
            }
            Self::Held { bpm } => format!("Heart rate {bpm:.0}. Keep it this light."),
        }
    }

    fn kind(&self) -> usize {
        match self {
            Self::Pace { .. } | Self::Held { .. } => 0,
            Self::RateUp { .. } | Self::RateDown { .. } => 1,
            Self::Harder { .. } | Self::Easier { .. } => 2,
            Self::Halfway | Self::Time | Self::Go { .. } | Self::Rest | Self::Soon => 3,
        }
    }
}

/// Strokes the median is taken over.
pub const WINDOW: usize = 6;
/// No corrections in the first part of a piece, while rate and HR settle.
pub const SETTLE_S: f64 = 60.0;
/// Nor in the first part of a work interval, while rate and power come up.
pub const WORK_SETTLE_S: f64 = 30.0;
/// The call before each work interval after the first comes this early.
pub const SOON_S: f64 = 10.0;
/// Heart rate over the ceiling this long: a step lighter.
pub const PACE_OVER_S: f64 = 30.0;
/// At most one step this often: heart rate follows the work a minute or so
/// late, and quicker steps would overshoot it.
pub const PACE_EVERY_S: f64 = 75.0;
/// A step lighter takes this share of the power off (at least 5 W), to the
/// nearest 5 W.
pub const PACE_DOWN: f64 = 0.10;
/// The pace's power never goes under this: lighter is paddling, and the
/// stroke loses its hold on the flywheel.
pub const WATTS_FLOOR: f64 = 30.0;
/// The power band round the pace, either side, for a part without its own,
/// and never narrower than this either side.
pub const PACE_BAND: f64 = 0.10;
pub const PACE_BAND_MIN_W: f64 = 5.0;
/// "Rate up" and "harder" only with heart rate at least this far under the
/// ceiling: over it is the pace's to answer, and close to it there's no room.
pub const ROOM_BPM: f64 = 5.0;

/// A step lighter than `w` watts; `None` at the floor.
pub fn lighter(w: f64) -> Option<f64> {
    (w > WATTS_FLOOR).then(|| (((w - (w * PACE_DOWN).max(5.0)) / 5.0).round() * 5.0).max(WATTS_FLOOR))
}

/// The pace of a part with a heart-rate ceiling (easy and steady rows, the
/// 4×4's work). Its power is the part's own, held all the way (an easy
/// row's is set from how far heart rate drifted on the last one,
/// `workout::easy_power`), and the ceiling is a guardrail: heart rate over
/// it for `PACE_OVER_S` takes a step lighter and a stroke slower, at most
/// every `PACE_EVERY_S`, down to a floor. Without a power of its own the
/// first step sets one from what the rower is pulling. It never steps back
/// up within the row: one power held is what lets the row's drift say
/// whether that power was aerobic. The rate stays inside its band. Tests
/// and fixed-rate work (no ceiling) keep their targets.
#[derive(Debug, Clone, Copy)]
struct Pace {
    rate: Option<PaceRate>,
    watts: Option<f64>,
    over_since: Option<i64>,
    changed_us: Option<i64>,
    /// As light as it goes with heart rate still over: said once.
    held: bool,
}

/// The rate to row at and the lowest it goes.
#[derive(Debug, Clone, Copy)]
struct PaceRate {
    now: f64,
    floor: f64,
}

impl Pace {
    fn new(t: &Targets) -> Option<Self> {
        t.hr_max?;
        let rate = t.rate.map(|(lo, hi)| {
            let start = ((lo + hi) / 2.0).round();
            PaceRate {
                now: start,
                floor: lo.ceil().min(start),
            }
        });
        Some(Self {
            rate,
            watts: t.watts.map(|(lo, hi)| (lo + hi) / 2.0),
            over_since: None,
            changed_us: None,
            held: false,
        })
    }

    /// The median heart rate of the last strokes against the ceiling, with
    /// the power the rower is pulling.
    fn update(&mut self, hr: f64, max: f64, pulling: Option<f64>, now: i64) -> Option<Cue> {
        if hr <= max {
            self.over_since = None;
            return None;
        }
        let held = |since: i64| (now - since) as f64 / 1e6;
        let over = *self.over_since.get_or_insert(now);
        if held(over) < PACE_OVER_S || self.changed_us.is_some_and(|t| held(t) < PACE_EVERY_S) {
            return None;
        }
        self.over_since = Some(now);
        self.changed_us = Some(now);
        let watts = self.watts.or(pulling).and_then(lighter);
        let rate = self.rate.filter(|r| r.now > r.floor).map(|r| r.now - 1.0);
        if watts.is_none() && rate.is_none() {
            return (!std::mem::replace(&mut self.held, true)).then_some(Cue::Held { bpm: hr });
        }
        if watts.is_some() {
            self.watts = watts;
        }
        if let (Some(r), Some(p)) = (rate, self.rate.as_mut()) {
            p.now = r;
        }
        Some(Cue::Pace { bpm: hr, watts, rate })
    }
}
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

/// Paces a workout on its own clock from Go (the row screen's Start): its
/// calls as each part begins, and corrections against the targets of the
/// part the rower is in. Nothing before Go: that's the warm-up.
#[derive(Debug)]
pub struct Pacer {
    workout: Workout,
    recent: VecDeque<Recent>,
    last_any_us: Option<i64>,
    last_kind_us: [Option<i64>; 4],
    go_us: Option<i64>,
    /// How many of `calls` have been made.
    called: usize,
    /// The part the last stroke was in: a new part starts a fresh median.
    part: Option<(Stage, u32)>,
    /// This part's pace, when it adapts to heart rate.
    pace: Option<Pace>,
}

impl Pacer {
    pub fn new(workout: Workout) -> Self {
        Self {
            workout,
            recent: VecDeque::with_capacity(WINDOW),
            last_any_us: None,
            last_kind_us: [None; 4],
            go_us: None,
            called: 0,
            part: None,
            pace: None,
        }
    }

    pub fn workout(&self) -> &Workout {
        &self.workout
    }

    /// Go: the workout's clock starts. Once.
    pub fn start(&mut self, go_us: i64) {
        self.go_us.get_or_insert(go_us);
    }

    pub fn started(&self) -> bool {
        self.go_us.is_some()
    }

    /// Where the workout is; `None` before Go.
    pub fn position(&self, now_us: i64) -> Option<Position> {
        self.go_us.map(|go| self.workout.at((now_us - go) as f64 / 1e6))
    }

    /// Started and not over.
    pub fn running(&self, now_us: i64) -> bool {
        self.position(now_us).is_some_and(|p| p.stage != Stage::Done)
    }

    /// The part's own targets; none in a rest, before Go or after the end.
    fn part_targets(&self, now_us: i64) -> Option<&Targets> {
        let p = self.position(now_us)?;
        match (&self.workout, p.stage) {
            (Workout::Piece { targets, .. }, Stage::Work) => Some(targets),
            (Workout::Intervals { work, .. }, Stage::Work) => work.get(p.n as usize - 1),
            _ => None,
        }
    }

    /// What the rower aims for now: the piece's or the work interval's
    /// targets, the rate and power bands round the pace.
    pub fn targets(&self, now_us: i64) -> Option<Targets> {
        let mut t = self.part_targets(now_us)?.clone();
        let Some(pace) = self.pace else { return Some(t) };
        if let (Some(r), Some((lo, hi))) = (pace.rate, t.rate) {
            let half = (hi - lo) / 2.0;
            t.rate = Some(((r.now - half).max(lo), (r.now + half).min(hi)));
        }
        if let Some(w) = pace.watts {
            // As wide as the part's own band, else PACE_BAND either side; at
            // least PACE_BAND_MIN_W, or a light row's strokes would stray
            // out of it one by one.
            let k = t.watts.map_or(PACE_BAND, |(lo, hi)| (hi - lo) / (hi + lo));
            let half = (w * k).max(PACE_BAND_MIN_W);
            t.watts = Some(((w - half).round(), (w + half).round()));
        }
        Some(t)
    }

    /// The rhythm to row at now: the pace's, else the targets' own.
    pub fn rhythm(&self, now_us: i64) -> Option<Rhythm> {
        let own = self.part_targets(now_us)?;
        match self.pace.and_then(|p| p.rate) {
            Some(r) => Some(Rhythm::at(r.now)),
            None => own.rhythm(),
        }
    }

    /// Every call through the workout, seconds after Go, in order.
    fn calls(&self) -> Vec<(f64, Cue)> {
        let mid = |b: Option<(f64, f64)>| b.map(|(lo, hi)| (lo + hi) / 2.0);
        let go = |n: u32, of: u32, t: &Targets| Cue::Go {
            n,
            of,
            rate: mid(t.rate),
            watts: mid(t.watts),
        };
        match &self.workout {
            Workout::Piece { minutes, targets } => {
                let mut c = vec![(0.0, go(1, 1, targets))];
                if let Some(m) = *minutes {
                    c.extend([(m * 30.0, Cue::Halfway), (m * 60.0, Cue::Time)]);
                }
                c
            }
            Workout::Intervals { work_s, rest_s, work } => {
                let (w, r, of) = (*work_s as f64, *rest_s as f64, work.len() as u32);
                let mut c = Vec::new();
                for (k, t) in (0u32..).zip(work) {
                    let at = k as f64 * (w + r);
                    if k > 0 {
                        c.push((at - SOON_S, Cue::Soon));
                    }
                    c.push((at, go(k + 1, of, t)));
                    c.push((at + w, if k + 1 == of { Cue::Time } else { Cue::Rest }));
                }
                c
            }
        }
    }

    /// Every second or so: the call that's due, if one is.
    pub fn tick(&mut self, now_us: i64) -> Option<Cue> {
        let elapsed = (now_us - self.go_us?) as f64 / 1e6;
        let (at, cue) = *self.calls().get(self.called)?;
        if elapsed < at {
            return None;
        }
        self.called += 1;
        Some(self.said(cue, now_us))
    }

    /// Feed each completed stroke; returns at most one correction.
    pub fn stroke(&mut self, s: &StrokeRecord) -> Option<Cue> {
        let now = s.t_next_catch_us.unwrap_or(s.t_catch_us);
        let p = self.position(now)?;
        if self.part != Some((p.stage, p.n)) {
            self.part = Some((p.stage, p.n));
            // A median across two parts would say nothing about either.
            self.recent.clear();
            // Each part starts at its own pace.
            self.pace = self.part_targets(now).and_then(Pace::new);
        }
        if self.recent.len() == WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(Recent {
            rate: s.rate().or(s.spm.map(f64::from)),
            watts: s.watts.map(f64::from),
            hr: s.hr_bpm,
        });
        let into_part = match (&self.workout, self.go_us) {
            (Workout::Intervals { work_s, rest_s, .. }, Some(go)) => {
                (now - go) as f64 / 1e6 - (p.n - 1) as f64 * (*work_s + *rest_s) as f64
            }
            (_, go) => (now - go.unwrap_or(now)) as f64 / 1e6,
        };
        let settle = match self.workout {
            Workout::Piece { .. } => SETTLE_S,
            Workout::Intervals { .. } => WORK_SETTLE_S,
        };
        if p.stage != Stage::Work || into_part < settle || self.recent.len() < WINDOW {
            return None;
        }
        // A change of pace, for heart rate, gets through: it comes at most
        // once every `PACE_EVERY_S`.
        let max = self.part_targets(now).and_then(|t| t.hr_max);
        let pulling = self.median(|r| r.watts);
        if let (Some(max), Some(hr)) = (max, self.median(|r| r.hr))
            && let Some(cue) = self.pace.as_mut().and_then(|pace| pace.update(hr, max, pulling, now))
        {
            return Some(self.said(cue, now));
        }
        if self.last_any_us.is_some_and(|t| (now - t) as f64 / 1e6 < COOLDOWN_S) {
            return None;
        }
        let cue = self.correction(now)?;
        let repeat_ok = self.last_kind_us[cue.kind()].is_none_or(|t| (now - t) as f64 / 1e6 >= REPEAT_S);
        repeat_ok.then(|| self.said(cue, now))
    }

    /// The median of the last strokes' values, when at least half have one.
    fn median(&self, pick: fn(&Recent) -> Option<f64>) -> Option<f64> {
        let mut xs: Vec<f64> = self.recent.iter().filter_map(pick).collect();
        if xs.len() < WINDOW / 2 {
            return None;
        }
        xs.sort_by(f64::total_cmp);
        Some(xs[xs.len() / 2])
    }

    /// Rate first, then power.
    fn correction(&self, now_us: i64) -> Option<Cue> {
        let median = |pick: fn(&Recent) -> Option<f64>| self.median(pick);
        let t = self.targets(now_us)?;
        let room = match (t.hr_max, median(|r| r.hr)) {
            (Some(max), Some(hr)) => hr <= max - ROOM_BPM,
            _ => true,
        };
        if let (Some((lo, hi)), Some(rate)) = (t.rate, median(|r| r.rate)) {
            let target = (lo + hi) / 2.0;
            if rate < lo && room {
                return Some(Cue::RateUp { target });
            }
            if rate > hi {
                return Some(Cue::RateDown { target });
            }
        }
        if let (Some((lo, hi)), Some(w)) = (t.watts, median(|r| r.watts)) {
            let target = (lo + hi) / 2.0;
            if w < lo && room {
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

    /// Go at the first stroke, a tick as each stroke ends, then the stroke:
    /// every call and correction, at the time it came.
    fn run(workout: Workout, strokes: &[StrokeRecord]) -> Vec<(f64, Cue)> {
        let mut p = Pacer::new(workout);
        p.start(T0);
        let mut out = Vec::new();
        for s in strokes {
            let end = s.t_next_catch_us.unwrap_or(s.t_catch_us);
            let at = (end - T0) as f64 / 1e6;
            out.extend(p.tick(end).map(|c| (at, c)));
            out.extend(p.stroke(s).map(|c| (at, c)));
        }
        out
    }

    /// The corrections only, without the calls.
    fn fixes(cues: Vec<(f64, Cue)>) -> Vec<(f64, Cue)> {
        cues.into_iter().filter(|(_, c)| c.kind() != 3).collect()
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
    fn on_target_only_hears_the_calls() {
        let cues = run(reference(), &steady(420, 3.0, 53.0, |_| 140.0));
        let kinds: Vec<_> = cues.iter().map(|(_, c)| *c).collect();
        let go = Cue::Go { n: 1, of: 1, rate: Some(20.0), watts: Some(53.0) };
        assert_eq!(kinds, vec![go, Cue::Halfway, Cue::Time]);
        assert!((cues[1].0 - 600.0).abs() < 3.1, "halfway at {}", cues[1].0);
        assert_eq!(go.text(), "Go. Rate 20. 53 watts.");
    }

    #[test]
    fn nothing_before_go() {
        let mut p = Pacer::new(reference());
        for s in &steady(60, 2.5, 90.0, |_| 140.0) {
            assert_eq!(p.stroke(s), None, "a warm-up: no corrections");
            assert_eq!(p.tick(s.t_catch_us), None);
        }
        assert!(p.position(T0).is_none() && p.targets(T0).is_none() && !p.running(T0));
    }

    #[test]
    fn rating_high_is_corrected_after_settling_with_cooldowns() {
        // 24 spm the whole way: too fast.
        let cues = fixes(run(reference(), &steady(120, 2.5, 53.0, |_| 140.0)));
        let first = cues[0];
        assert!(matches!(first.1, Cue::RateDown { target } if target == 20.0));
        assert!(first.0 >= SETTLE_S, "no cue while settling: {}", first.0);
        // Same correction no more than once a minute.
        for pair in cues.windows(2) {
            assert!(pair[1].0 - pair[0].0 >= REPEAT_S - 2.5);
        }
        assert!(cues.len() >= 3);
    }

    #[test]
    fn heart_rate_outranks_power() {
        let t = Targets {
            rate: Some((19.0, 21.0)),
            watts: Some((48.0, 58.0)),
            hr_max: Some(150.0),
        };
        // Power and rate are low too, but "harder" or "rate up" with heart
        // rate over the ceiling would be the wrong advice: only the pace
        // speaks, lighter each time.
        let cues = fixes(run(piece(None, t.clone()), &steady(60, 3.5, 40.0, |_| 160.0)));
        assert!(!cues.is_empty());
        assert!(cues.iter().all(|(_, c)| matches!(c, Cue::Pace { .. })), "{cues:?}");
        // With heart rate fine, rate and power get their turn.
        let cues = fixes(run(piece(None, t.clone()), &steady(60, 3.5, 53.0, |_| 140.0)));
        assert!(matches!(cues[0].1, Cue::RateUp { target } if target == 20.0));
        let cues = fixes(run(piece(None, t), &steady(60, 3.0, 40.0, |_| 140.0)));
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
        assert!(fixes(run(piece(None, t), &s)).is_empty());
    }

    #[test]
    fn texts() {
        assert_eq!(Cue::RateUp { target: 20.0 }.text(), "Rate up. 20.");
        let pace = |watts, rate| Cue::Pace { bpm: 162.4, watts, rate }.text();
        assert_eq!(pace(Some(55.0), Some(18.0)), "Heart rate 162. Lighter: 55 watts, rate 18.");
        assert_eq!(pace(Some(50.0), None), "Heart rate 162. Lighter: 50 watts.");
        assert_eq!(pace(None, Some(18.0)), "Heart rate 162. Slower: rate 18.");
        assert_eq!(Cue::Held { bpm: 158.0 }.text(), "Heart rate 158. Keep it this light.");
        assert!(Targets::default().is_empty());
        let go = |n, rate, watts| Cue::Go { n, of: 4, rate, watts }.text();
        assert_eq!(go(1, None, None), "Go. 1 of 4.");
        assert_eq!(go(2, Some(22.0), Some(110.0)), "Go. 2 of 4. Rate 22. 110 watts.");
        assert_eq!(go(4, None, None), "Last one.");
        assert_eq!(Cue::Soon.text(), "Ten seconds.");
    }

    mod intervals {
        use super::super::*;
        use crate::session::tests::{T0, steady};

        const S: i64 = 1_000_000;

        /// Two work intervals of 4 min with 3 min rest between, from Go at
        /// T0, rowed at `spm` and `watts` throughout: a tick a second and
        /// each stroke as it ends, in time order.
        fn row(work: Vec<Targets>, spm: f64, watts: f64, hr: f64) -> (Pacer, Vec<(f64, Cue)>) {
            let mut p = Pacer::new(Workout::Intervals { work_s: 240, rest_s: 180, work });
            p.start(T0);
            let n = (60.0 * 17.0 / (60.0 / spm)) as u32;
            let strokes = steady(n, 60.0 / spm, watts, |_| hr);
            let mut cues = Vec::new();
            let mut next = strokes.iter().peekable();
            for sec in 0..17 * 60 {
                let t = T0 + sec * S;
                cues.extend(p.tick(t).map(|c| (sec as f64, c)));
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
        fn calls_each_interval_on_time_and_ends_after_the_last() {
            let (p, cues) = row(hard(2), 26.0, 110.0, 160.0);
            let calls: Vec<(f64, Cue)> = cues.into_iter().filter(|(_, c)| c.kind() == 3).collect();
            let go = |n| Cue::Go { n, of: 2, rate: Some(26.0), watts: None };
            assert_eq!(calls, vec![(0.0, go(1)), (240.0, Cue::Rest), (410.0, Cue::Soon), (420.0, go(2)), (660.0, Cue::Time)]);
            let end = T0 + 700 * S;
            assert!(p.targets(end).is_none() && !p.running(end), "nothing to aim for once it's over");
            assert!(p.targets(T0 + 300 * S).is_none(), "nor in a rest");
            assert!(p.running(T0 + 300 * S), "a rest is part of it");
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
            let first = cues.iter().find(|(_, c)| matches!(c, Cue::Pace { .. })).map(|(_, c)| *c);
            assert_eq!(first, Some(Cue::Pace { bpm: 180.0, watts: Some(100.0), rate: Some(25.0) }));
        }
    }

    mod pace {
        use super::super::*;
        use crate::session::tests::{T0, steady};
        use crate::workout::Kind;

        const S: i64 = 1_000_000;

        fn easy() -> Workout {
            Workout::Piece {
                minutes: Some(30.0),
                targets: Targets {
                    rate: Some((17.5, 20.5)),
                    watts: None,
                    hr_max: Some(154.0),
                },
            }
        }

        /// Strokes 3 s apart from Go at T0, heart rate by the second.
        fn row(w: Workout, minutes: u32, hr: impl Fn(f64) -> f64) -> (Pacer, Vec<(f64, Cue)>) {
            let mut p = Pacer::new(w);
            p.start(T0);
            let strokes = steady(minutes * 20, 3.0, 60.0, |i| hr(i as f64 * 3.0));
            let cues = strokes
                .iter()
                .filter_map(|s| p.stroke(s).map(|c| ((s.t_next_catch_us.unwrap() - T0) as f64 / 1e6, c)))
                .filter(|(_, c)| matches!(c, Cue::Pace { .. } | Cue::Held { .. }))
                .collect();
            (p, cues)
        }

        /// Each change of pace: its power and rate.
        fn steps(cues: &[(f64, Cue)]) -> Vec<(Option<f64>, Option<f64>)> {
            cues.iter()
                .filter_map(|(_, c)| match *c {
                    Cue::Pace { watts, rate, .. } => Some((watts, rate)),
                    _ => None,
                })
                .collect()
        }

        #[test]
        fn heart_rate_held_over_the_ceiling_steps_the_pace_down_to_its_floor_and_says_so_once() {
            let (p, cues) = row(easy(), 14, |_| 160.0);
            let (at, first) = cues[0];
            // Rowed at 60 W with no power to aim for: the first step sets one.
            assert_eq!(first, Cue::Pace { bpm: 160.0, watts: Some(55.0), rate: Some(18.0) });
            assert_eq!(first.text(), "Heart rate 160. Lighter: 55 watts, rate 18.");
            assert!(at >= SETTLE_S + PACE_OVER_S - 3.0, "not before it's been over a while: {at}");
            let watts: Vec<f64> = steps(&cues).iter().filter_map(|s| s.0).collect();
            assert_eq!(watts, [55.0, 50.0, 45.0, 40.0, 35.0, 30.0], "a step lighter at a time, to the floor");
            assert!(cues.windows(2).all(|w| w[1].0 - w[0].0 >= PACE_EVERY_S - 3.0), "{cues:?}");
            let held: Vec<_> = cues.iter().filter(|(_, c)| matches!(c, Cue::Held { .. })).collect();
            assert_eq!(held.len(), 1, "then once that it's as light as it goes: {cues:?}");
            assert!(matches!(cues.last().unwrap().1, Cue::Held { .. }));
            let now = T0 + 800 * S;
            assert_eq!(p.rhythm(now).map(|r| r.rate), Some(18.0), "the ghost rows the new pace");
            let t = p.targets(now).unwrap();
            assert_eq!(t.rate, Some((17.5, 19.5)), "the band moves with it, inside its own");
            assert_eq!(t.watts, Some((25.0, 35.0)), "never narrower than 5 W either side");
        }

        #[test]
        fn never_steps_back_up_within_a_row() {
            let mut w = easy();
            w.targets_mut()[0].watts = Some((36.0, 44.0));
            // Over the ceiling for the first 3 minutes, well under after.
            let (p, cues) = row(w, 12, |t| if t < 180.0 { 160.0 } else { 140.0 });
            assert_eq!(steps(&cues), [(Some(35.0), Some(18.0)), (Some(30.0), None)], "only ever lighter: {cues:?}");
            let t = p.targets(T0 + 700 * S).unwrap();
            assert_eq!((t.watts, p.rhythm(T0 + 700 * S).map(|r| r.rate)), (Some((25.0, 35.0)), Some(18.0)));
        }

        #[test]
        fn an_easy_row_with_a_power_starts_there() {
            let mut w = easy();
            w.targets_mut()[0].watts = Some((36.0, 44.0));
            let (_, cues) = row(w, 4, |_| 160.0);
            assert_eq!(steps(&cues)[0], (Some(35.0), Some(18.0)), "a step under 40 W, not under what was pulled");
        }

        #[test]
        fn never_rate_up_or_harder_while_heart_rate_is_over() {
            let mut p = Pacer::new(easy());
            p.start(T0);
            // Rate 16, under the band, all the way to the floor and past it.
            let cues: Vec<Cue> = steady(300, 3.75, 25.0, |_| 165.0).iter().filter_map(|s| p.stroke(s)).collect();
            assert!(cues.iter().all(|c| matches!(c, Cue::Pace { .. } | Cue::Held { .. })), "{cues:?}");
        }

        #[test]
        fn a_test_keeps_its_rate_whatever_the_heart_rate() {
            let (p, cues) = row(Kind::Reference.workout(None, None), 8, |_| 175.0);
            assert!(cues.is_empty());
            assert_eq!(p.rhythm(T0 + 300 * S).map(|r| r.rate), Some(20.0));
        }

        #[test]
        fn a_four_by_four_steps_down_near_its_ceiling_and_starts_each_interval_afresh() {
            let h = crate::zones::HeartProfile::new(73.0, 188.0, None).unwrap();
            let mut p = Pacer::new(Kind::FourByFour.workout(Some(&h), None));
            p.start(T0);
            // Each work interval rowed at rate 23, heart rate 182 (over 179) throughout.
            let strokes: Vec<StrokeRecord> = steady(4 * 25 * 7, 60.0 / 23.0, 100.0, |_| 182.0)
                .into_iter()
                .filter(|s| p.position(s.t_next_catch_us.unwrap()).is_some_and(|x| x.stage == Stage::Work))
                .collect();
            let mut paces = Vec::new();
            for s in &strokes {
                if let Some(Cue::Pace { rate: Some(rate), .. }) = p.stroke(s) {
                    let n = p.position(s.t_next_catch_us.unwrap()).unwrap().n;
                    paces.push((n, rate));
                }
            }
            assert_eq!(&paces[..2], [(1, 22.0), (1, 21.0)], "{paces:?}");
            assert!(paces.iter().any(|&(n, r)| n == 2 && r == 22.0), "the next interval starts at 23 again: {paces:?}");
            assert!(paces.iter().all(|&(_, r)| r >= 20.0), "never under the band");
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
