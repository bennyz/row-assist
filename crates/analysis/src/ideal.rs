//! The stroke to aim for, as a body: where the legs, the body and the arms
//! should be for where the handle is (DESIGN.md §5.4).
//!
//! A stroke is three movements in a fixed order. Drive: the legs push, then
//! the body swings back, then the arms draw. Recovery: arms away, body over,
//! then the slide. Each is a [`Parts`] fraction from its catch position (0)
//! to its finish position (1), and [`path`] is the order they should move in.
//!
//! The target is matched to the rower by handle position, not by time: with
//! the handle here, the legs, body and arms should be there. That holds at
//! any speed and standing still, so it shows the shape of the stroke at every
//! point of it; timing is the rhythm's job (`cues::Rhythm`).
//!
//! Figures are the mirror's: thigh lengths from where the feet rest, x toward
//! the feet, y up.

use serde::{Deserialize, Serialize};

use crate::pose::Joint;
use crate::technique::{CATCH_TRUNK_GUIDE, FINISH_TRUNK_GUIDE, Figure};

/// Knee angle at the finish: legs long, not locked.
const FINISH_KNEE_DEG: f64 = 172.0;
/// What a straight arm reads as: the camera never sees quite 180°.
const STRAIGHT_ELBOW_DEG: f64 = 170.0;
/// The handle finishes this far in front of the body, thigh lengths.
const RIBS_FORWARD: f64 = 0.15;
/// Ear above the shoulder when the ear isn't seen, thigh lengths.
const HEAD: f64 = 0.45;
/// Figures needed to measure a body.
pub const MIN_FIGURES: usize = 30;
/// The handle has turned round once it is this far back from its furthest
/// point, thigh lengths.
const TURN: f64 = 0.2;
/// A part this far from where it should be (as a share of its travel) is
/// off. The arms get more room: an elbow's angle moves a long way for a
/// small move of the hands while the arm is nearly straight.
pub const TOLERANCE: Parts = Parts {
    legs: 0.2,
    body: 0.2,
    arms: 0.25,
};

/// How far each part is from its catch position (0) to its finish position
/// (1): the seat along the rail, the body's lean, the elbow's bend.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Parts {
    pub legs: f64,
    pub body: f64,
    pub arms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Drive,
    Recovery,
}

/// The step of the stroke: which part should be moving now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Legs,
    Body,
    Arms,
    ArmsAway,
    BodyOver,
    Slide,
}

fn ramp(u: f64, from: f64, to: f64) -> f64 {
    let x = ((u - from) / (to - from)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The good stroke at `u` of a phase (0 its start, 1 its end). Drive: the
/// legs, the body from when they are half down, the arms once they are down.
/// Recovery: the arms, the body close behind, and the knees only once the
/// hands are away and the body is nearly over.
pub fn path(phase: Phase, u: f64) -> Parts {
    match phase {
        Phase::Drive => Parts {
            legs: ramp(u, 0.0, 0.6),
            body: ramp(u, 0.3, 0.8),
            arms: ramp(u, 0.6, 1.0),
        },
        Phase::Recovery => Parts {
            arms: 1.0 - ramp(u, 0.0, 0.2),
            body: 1.0 - ramp(u, 0.1, 0.35),
            legs: 1.0 - ramp(u, 0.3, 1.0),
        },
    }
}

/// The step at `u` of a phase: the part [`path`] starts moving there.
pub fn step(phase: Phase, u: f64) -> Step {
    match phase {
        Phase::Drive if u < 0.3 => Step::Legs,
        Phase::Drive if u < 0.6 => Step::Body,
        Phase::Drive => Step::Arms,
        Phase::Recovery if u < 0.1 => Step::ArmsAway,
        Phase::Recovery if u < 0.3 => Step::BodyOver,
        Phase::Recovery => Step::Slide,
    }
}

type P = (f64, f64);

fn pt(f: &Figure, j: Joint) -> Option<P> {
    f.get(j as usize).copied().flatten().map(|[x, y]| (x as f64, y as f64))
}

fn dist(a: P, b: P) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied()
}

/// The middle joint of a chain from `a` to `c` (lengths `l1` then `l2`): the
/// higher of the two solutions if `up`.
fn middle(a: P, c: P, l1: f64, l2: f64, up: bool) -> P {
    let (dx, dy) = (c.0 - a.0, c.1 - a.1);
    let d = dx.hypot(dy).clamp(1e-6, l1 + l2 - 1e-6);
    let along = (l1 * l1 - l2 * l2 + d * d) / (2.0 * d);
    let h = (l1 * l1 - along * along).max(0.0).sqrt();
    let (mx, my) = (a.0 + along * dx / d, a.1 + along * dy / d);
    let (p, q) = ((mx - h * dy / d, my + h * dx / d), (mx + h * dy / d, my - h * dx / d));
    if (p.1 >= q.1) == up { p } else { q }
}

/// Angle at `b` between `a` and `c`, degrees.
fn angle_at(a: P, b: P, c: P) -> f64 {
    let (ux, uy, vx, vy) = (a.0 - b.0, a.1 - b.1, c.0 - b.0, c.1 - b.1);
    (ux * vy - uy * vx).abs().atan2(ux * vx + uy * vy).to_degrees()
}

/// The elbow angle that puts the hands `chord` from the shoulder.
fn elbow_for(chord: f64, upper: f64, fore: f64) -> f64 {
    ((upper * upper + fore * fore - chord * chord) / (2.0 * upper * fore)).clamp(-1.0, 1.0).acos().to_degrees()
}

/// One rower's proportions and where their erg puts them: enough to draw the
/// good stroke on their own body.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    thigh: f64,
    shin: f64,
    trunk: f64,
    upper: f64,
    fore: f64,
    head: f64,
    /// Heights of the hip and of the handle: both run level.
    seat: f64,
    handle: f64,
    ankle: P,
    heel: Option<P>,
    toe: Option<P>,
    /// Body angle at the catch and the finish, degrees, + toward the feet.
    catch_lean: f64,
    finish_lean: f64,
    /// Seat position with the shins vertical, and with the legs long.
    catch_x: f64,
    finish_x: f64,
    /// Elbow angle with the handle at the ribs.
    finish_elbow: f64,
}

impl Body {
    /// From figures of the rower (any part of the stroke, or sitting still),
    /// leaning to the middle of the guides until [`Body::set_lean`] says how
    /// far they lean themselves.
    pub fn measure<'a>(figures: impl IntoIterator<Item = &'a Figure>) -> Option<Self> {
        use Joint::*;
        let seen: Vec<&Figure> = figures
            .into_iter()
            .filter(|f| crate::pose::CORE.iter().all(|&j| pt(f, j).is_some()))
            .collect();
        if seen.len() < MIN_FIGURES {
            return None;
        }
        let seg = |a: Joint, b: Joint| median(seen.iter().filter_map(|f| Some(dist(pt(f, a)?, pt(f, b)?))).collect());
        let coord = |j: Joint, pick: fn(P) -> f64| median(seen.iter().filter_map(|f| pt(f, j).map(pick)).collect());
        let point = |j: Joint| Some((coord(j, |p| p.0)?, coord(j, |p| p.1)?));
        let (thigh, shin, trunk) = (seg(Hip, Knee)?, seg(Knee, Ankle)?, seg(Hip, Shoulder)?);
        let (upper, fore) = (seg(Shoulder, Elbow)?, seg(Elbow, Wrist)?);
        let (seat, handle, ankle) = (coord(Hip, |p| p.1)?, coord(Wrist, |p| p.1)?, point(Ankle)?);

        // Catch: the knee over the ankle. Finish: the legs long.
        let height = seat - ankle.1;
        let rise = shin - height;
        let catch_x = ankle.0 - (thigh * thigh - rise * rise).max(0.0).sqrt();
        let leg = (thigh * thigh + shin * shin - 2.0 * thigh * shin * FINISH_KNEE_DEG.to_radians().cos()).sqrt();
        let finish_x = ankle.0 - (leg * leg - height * height).max(0.0).sqrt();
        // A hip the camera puts above the knee's reach, or such: no stroke to draw.
        if finish_x >= catch_x - 0.3 {
            return None;
        }
        let mut body = Self {
            thigh,
            shin,
            trunk,
            upper,
            fore,
            // The ear is often hidden by the shoulder: not needed to measure.
            head: seg(Shoulder, Ear).unwrap_or(HEAD),
            seat,
            handle,
            ankle,
            heel: point(Heel),
            toe: point(Toe),
            catch_lean: 0.0,
            finish_lean: 0.0,
            catch_x,
            finish_x,
            finish_elbow: 0.0,
        };
        body.set_lean(None, None);
        Some(body)
    }

    /// The rower's own body angle at the catch and at the finish (degrees,
    /// positive toward the feet), when known. The target leans as they do where
    /// that is inside the guides and to the nearest guide where it isn't, so
    /// only what is off shows as a difference; the middle of a guide when
    /// not known.
    pub fn set_lean(&mut self, catch: Option<f64>, finish: Option<f64>) {
        let within = |own: Option<f64>, (lo, hi): (f64, f64)| own.map_or((lo + hi) / 2.0, |v| v.clamp(lo, hi));
        self.catch_lean = within(catch, CATCH_TRUNK_GUIDE);
        self.finish_lean = within(finish, FINISH_TRUNK_GUIDE);
        // The arms end with the handle just in front of the body, at its own height.
        let (s, c) = self.finish_lean.to_radians().sin_cos();
        let shoulder = (self.finish_x + self.trunk * s, self.seat + self.trunk * c);
        let ribs = (self.finish_x + (self.handle - self.seat) * s / c + RIBS_FORWARD, self.handle);
        self.finish_elbow = elbow_for(dist(shoulder, ribs), self.upper, self.fore);
    }

    /// Ear, shoulder, elbow, wrist, hip and knee with each part at `p`.
    fn joints(&self, p: Parts) -> [P; 6] {
        let hip = (self.catch_x + (self.finish_x - self.catch_x) * p.legs, self.seat);
        let knee = middle(self.ankle, hip, self.shin, self.thigh, true);
        let (s, c) = (self.catch_lean + (self.finish_lean - self.catch_lean) * p.body).to_radians().sin_cos();
        let shoulder = (hip.0 + self.trunk * s, hip.1 + self.trunk * c);
        let ear = (shoulder.0 + self.head * s, shoulder.1 + self.head * c);
        // The hands stay on the handle's line, as far out as the elbow lets them.
        let elbow_deg = STRAIGHT_ELBOW_DEG + (self.finish_elbow - STRAIGHT_ELBOW_DEG) * p.arms;
        let chord = (self.upper * self.upper + self.fore * self.fore
            - 2.0 * self.upper * self.fore * elbow_deg.to_radians().cos())
        .sqrt();
        let drop = shoulder.1 - self.handle;
        let wrist = (shoulder.0 + (chord * chord - drop * drop).max(0.0).sqrt(), self.handle);
        let elbow = middle(shoulder, wrist, self.upper, self.fore, false);
        [ear, shoulder, elbow, wrist, hip, knee]
    }

    /// The body with each part at `p`, as a figure.
    pub fn pose(&self, p: Parts) -> Figure {
        let [ear, shoulder, elbow, wrist, hip, knee] = self.joints(p);
        [Some(ear), Some(shoulder), Some(elbow), Some(wrist), Some(hip), Some(knee), Some(self.ankle), self.heel, self.toe]
            .into_iter()
            .map(|p| p.map(|(x, y)| [x as f32, y as f32]))
            .collect()
    }

    /// Where each part of a figure is between this body's catch and finish
    /// positions. Past either end counts as that end: how far the stroke
    /// reaches is judged at the catch and the finish, not here. `None` when
    /// a joint it needs isn't seen.
    pub fn parts(&self, fig: &Figure) -> Option<Parts> {
        let s = sample(fig)?;
        let share = |v: f64, from: f64, to: f64| ((v - from) / (to - from)).clamp(0.0, 1.0);
        Some(Parts {
            legs: share(s.seat, self.catch_x, self.finish_x),
            body: share(s.lean, self.catch_lean, self.finish_lean),
            arms: share(s.elbow, STRAIGHT_ELBOW_DEG, self.finish_elbow),
        })
    }

    /// How far through `phase` the good stroke has the handle at `handle_x`:
    /// 0 at its start, 1 at its end, the nearer of them when the handle is
    /// outside the good stroke's travel.
    pub fn progress(&self, phase: Phase, handle_x: f64) -> f64 {
        // The handle only goes back in the drive and forward in the recovery.
        let before = |u: f64| {
            let x = self.joints(path(phase, u))[3].0;
            match phase {
                Phase::Drive => x > handle_x,
                Phase::Recovery => x < handle_x,
            }
        };
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if before(mid) { lo = mid } else { hi = mid }
        }
        (lo + hi) / 2.0
    }

    /// What the rower should see for one frame of themselves.
    pub fn guide(&self, phase: Phase, fig: &Figure) -> Option<Guide> {
        let u = self.progress(phase, pt(fig, Joint::Wrist)?.0);
        let (you, ideal) = (self.parts(fig)?, path(phase, u));
        Some(Guide {
            phase,
            step: step(phase, u),
            off: Off {
                legs: (you.legs - ideal.legs).abs() > TOLERANCE.legs,
                body: (you.body - ideal.body).abs() > TOLERANCE.body,
                arms: (you.arms - ideal.arms).abs() > TOLERANCE.arms,
            },
            you,
            ideal,
            target: self.pose(ideal),
        })
    }

    /// The good stroke's body at the catch and at the finish.
    pub fn ends(&self) -> (Figure, Figure) {
        (self.pose(path(Phase::Drive, 0.0)), self.pose(path(Phase::Drive, 1.0)))
    }

    /// When each part moves in the good stroke, on this body: what a
    /// stroke's own [`Sequence`] is held against.
    pub fn sequence(&self) -> Sequence {
        const STEPS: usize = 100;
        let through = |phase: Phase| -> Vec<Sample> {
            (0..=STEPS)
                .filter_map(|i| sample(&self.pose(path(phase, i as f64 / STEPS as f64))))
                .collect()
        };
        Sequence {
            drive: spans(&through(Phase::Drive)),
            recovery: spans(&through(Phase::Recovery)),
        }
    }
}

/// One frame against the good stroke: the target body with the handle where
/// the rower's is, and each part of theirs beside where it should be.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Guide {
    pub phase: Phase,
    pub step: Step,
    pub you: Parts,
    pub ideal: Parts,
    /// The parts more than [`TOLERANCE`] from where they should be.
    pub off: Off,
    pub target: Figure,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Off {
    pub legs: bool,
    pub body: bool,
    pub arms: bool,
}

/// Which way the handle is going, from where it has been: it turns round at
/// the catch and at the finish.
#[derive(Debug, Clone, Copy)]
pub struct Direction {
    phase: Phase,
    /// The furthest the handle has got this phase.
    extreme: Option<f64>,
}

impl Default for Direction {
    fn default() -> Self {
        // Sitting ready, the next move is the last of a recovery: up to the catch.
        Self {
            phase: Phase::Recovery,
            extreme: None,
        }
    }
}

/// What the handle did in one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moved {
    /// The phase it is in now.
    pub phase: Phase,
    /// It is as far as it has got this phase: the catch or the finish, if
    /// it turns round from here.
    pub furthest: bool,
    /// It has turned round: `phase` has just begun, at the last `furthest` frame.
    pub turned: bool,
}

impl Direction {
    /// The handle is now at `handle_x`.
    pub fn push(&mut self, handle_x: f64) -> Moved {
        let extreme = self.extreme.get_or_insert(handle_x);
        let (furthest, back) = match self.phase {
            Phase::Recovery => (handle_x >= *extreme, *extreme - handle_x),
            Phase::Drive => (handle_x <= *extreme, handle_x - *extreme),
        };
        let turned = !furthest && back > TURN;
        if turned {
            self.phase = match self.phase {
                Phase::Recovery => Phase::Drive,
                Phase::Drive => Phase::Recovery,
            };
        }
        if furthest || turned {
            *extreme = handle_x;
        }
        Moved {
            phase: self.phase,
            furthest,
            turned,
        }
    }
}

/// What one camera frame says about the three parts: where the handle and
/// the seat are (thigh lengths, + toward the feet), the body's lean
/// (degrees, + toward the feet) and the elbow's angle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub handle: f64,
    pub seat: f64,
    pub lean: f64,
    pub elbow: f64,
}

pub fn sample(fig: &Figure) -> Option<Sample> {
    use Joint::*;
    let (shoulder, elbow, wrist, hip) = (pt(fig, Shoulder)?, pt(fig, Elbow)?, pt(fig, Wrist)?, pt(fig, Hip)?);
    Some(Sample {
        handle: wrist.0,
        seat: hip.0,
        lean: (shoulder.0 - hip.0).atan2(shoulder.1 - hip.1).to_degrees(),
        elbow: angle_at(shoulder, elbow, wrist),
    })
}

/// Where in the handle's travel a part did its moving: 0 at the start of the
/// phase, 1 at its end.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

/// One phase of a stroke. A part that hardly moved (a drill, or a joint the
/// camera lost) has no span.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Spans {
    pub legs: Option<Span>,
    pub body: Option<Span>,
    pub arms: Option<Span>,
}

/// The order a stroke was rowed in: when each part moved, by the handle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Sequence {
    pub drive: Spans,
    pub recovery: Spans,
}

/// A part has started at this share of its own movement, and is done at `DONE`.
const STARTED: f64 = 0.15;
const DONE: f64 = 0.85;
/// Less movement than this isn't a stroke's worth: handle and seat in thigh
/// lengths, lean and elbow in degrees.
const MIN_HANDLE: f64 = 0.8;
const MIN_SEAT: f64 = 0.3;
const MIN_LEAN: f64 = 12.0;
const MIN_ELBOW: f64 = 30.0;

/// When each part moved through one phase, from its frames in order (the
/// first at the phase's start, the last at its end). Each part is measured
/// on its own travel this stroke, so a short stroke rowed in the right order
/// reads the same as a long one.
pub fn spans(samples: &[Sample]) -> Spans {
    let (Some(first), Some(last)) = (samples.first(), samples.last()) else {
        return Spans::default();
    };
    let travel = last.handle - first.handle;
    if travel.abs() < MIN_HANDLE {
        return Spans::default();
    }
    let span = |pick: fn(&Sample) -> f64, least: f64| -> Option<Span> {
        let (from, range) = (pick(first), pick(last) - pick(first));
        if range.abs() < least {
            return None;
        }
        // Handle travel at which the part first reaches `share` of its own.
        let reached = |share: f64| {
            let done = |s: &Sample| (pick(s) - from) / range;
            let at = |s: &Sample| (s.handle - first.handle) / travel;
            let i = samples.iter().position(|s| done(s) >= share)?;
            let Some(prev) = i.checked_sub(1).map(|i| &samples[i]) else {
                return Some(0.0);
            };
            let step = done(&samples[i]) - done(prev);
            let k = if step > 0.0 { (share - done(prev)) / step } else { 1.0 };
            Some((at(prev) + (at(&samples[i]) - at(prev)) * k).clamp(0.0, 1.0))
        };
        Some(Span {
            start: reached(STARTED)?,
            end: reached(DONE)?,
        })
    };
    Spans {
        legs: span(|s| s.seat, MIN_SEAT),
        body: span(|s| s.lean, MIN_LEAN),
        arms: span(|s| s.elbow, MIN_ELBOW),
    }
}

#[cfg(test)]
mod tests {
    use devices::sim_pose::{self, SimFault};

    use super::*;

    /// The simulator's rower as the mirror would draw it: thigh lengths from
    /// the ankle, x toward the feet, y up.
    fn sim_figure(phase: sim_pose::Phase, fault: Option<SimFault>) -> Figure {
        let j = sim_pose::figure(phase, fault);
        let ankle = j[Joint::Ankle as usize];
        let thigh = dist(j[Joint::Hip as usize], j[Joint::Knee as usize]);
        j.iter()
            .map(|&(x, y)| Some([((x - ankle.0) / thigh) as f32, ((ankle.1 - y) / thigh) as f32]))
            .collect()
    }

    fn sim_stroke(fault: Option<SimFault>) -> (Vec<Figure>, Vec<Figure>) {
        let steps = |phase: fn(f64) -> sim_pose::Phase| (0..=60).map(|i| sim_figure(phase(i as f64 / 60.0), fault)).collect();
        (steps(sim_pose::Phase::Drive), steps(sim_pose::Phase::Recovery))
    }

    fn sim_body() -> Body {
        let (drive, recovery) = sim_stroke(None);
        let mut body = Body::measure(drive.iter().chain(&recovery)).unwrap();
        body.set_lean(Some(25.0), Some(-25.0));
        body
    }

    fn sequence(fault: Option<SimFault>) -> Sequence {
        let (drive, recovery) = sim_stroke(fault);
        let samples = |figs: &[Figure]| figs.iter().filter_map(sample).collect::<Vec<_>>();
        Sequence {
            drive: spans(&samples(&drive)),
            recovery: spans(&samples(&recovery)),
        }
    }

    #[test]
    fn the_good_stroke_is_legs_body_arms_and_back() {
        for (phase, order) in [
            (Phase::Drive, [Step::Legs, Step::Body, Step::Arms]),
            (Phase::Recovery, [Step::ArmsAway, Step::BodyOver, Step::Slide]),
        ] {
            let steps: Vec<Step> = (0..=20).map(|i| step(phase, i as f64 / 20.0)).collect();
            let mut seen = steps.clone();
            seen.dedup();
            assert_eq!(seen, order);
        }
        // Each phase ends where the other starts.
        assert_eq!(path(Phase::Drive, 0.0), path(Phase::Recovery, 1.0));
        assert_eq!(path(Phase::Drive, 1.0), path(Phase::Recovery, 0.0));
        // The arms wait for the legs; the knees wait for the hands.
        let d = path(Phase::Drive, 0.5);
        assert!(d.legs > 0.9 && d.arms == 0.0, "{d:?}");
        let r = path(Phase::Recovery, 0.25);
        assert!(r.arms == 0.0 && r.legs == 1.0, "{r:?}");
    }

    #[test]
    fn the_target_is_the_rowers_own_body_at_catch_and_finish() {
        let body = sim_body();
        let (catch, finish) = body.ends();
        let knee = pt(&catch, Joint::Knee).unwrap();
        assert!(knee.0.abs() < 0.02, "shins vertical at the catch: {knee:?}");
        // Segments keep the rower's lengths wherever the parts are.
        for p in [path(Phase::Drive, 0.0), path(Phase::Drive, 0.45), path(Phase::Recovery, 0.2)] {
            let f = body.pose(p);
            let len = |a, b| dist(pt(&f, a).unwrap(), pt(&f, b).unwrap());
            assert!((len(Joint::Hip, Joint::Knee) - 1.0).abs() < 0.01);
            assert!((len(Joint::Knee, Joint::Ankle) - 1.0).abs() < 0.01);
            assert!((len(Joint::Shoulder, Joint::Elbow) - 125.0 / 175.0).abs() < 0.01);
        }
        // It lies on the simulator's clean rower at both ends.
        for (target, sim) in [
            (&catch, sim_figure(sim_pose::Phase::Drive(0.0), None)),
            (&finish, sim_figure(sim_pose::Phase::Drive(1.0), None)),
        ] {
            // (Not the hands: the target keeps them on the handle's level line.)
            for j in [Joint::Shoulder, Joint::Hip, Joint::Knee] {
                let d = dist(pt(target, j).unwrap(), pt(&sim, j).unwrap());
                assert!(d < 0.2, "{j:?} is {d:.2} thighs off");
            }
        }
    }

    #[test]
    fn a_clean_stroke_stays_on_the_target_all_the_way() {
        let body = sim_body();
        let (drive, recovery) = sim_stroke(None);
        for (phase, figs) in [(Phase::Drive, &drive), (Phase::Recovery, &recovery)] {
            for (i, fig) in figs.iter().enumerate() {
                let g = body.guide(phase, fig).unwrap();
                assert_eq!(g.off, Off::default(), "{phase:?} frame {i}: {:?} against {:?}", g.you, g.ideal);
                // The target's hands are where the rower's are.
                let (mine, target) = (pt(fig, Joint::Wrist).unwrap(), pt(&g.target, Joint::Wrist).unwrap());
                assert!((mine.0 - target.0).abs() < 0.05, "{phase:?} frame {i}: handle {mine:?} vs {target:?}");
            }
        }
    }

    #[test]
    fn a_fault_shows_in_its_part_where_it_happens() {
        let body = sim_body();
        // Arms bending from the catch: ahead of the target while the legs drive.
        let fig = sim_figure(sim_pose::Phase::Drive(0.3), Some(SimFault::EarlyArms));
        let g = body.guide(Phase::Drive, &fig).unwrap();
        assert!(g.off.arms && g.you.arms > g.ideal.arms, "arms {:.2} vs {:.2}", g.you.arms, g.ideal.arms);
        assert!(g.ideal.arms < 0.05, "the target's arms are still long");
        // The handle is matched, so the legs are behind by as much.
        assert!(g.you.legs < g.ideal.legs - 0.1, "legs {:.2} vs {:.2}", g.you.legs, g.ideal.legs);
        // Stopped short of full compression: the target's catch is further on.
        let fig = sim_figure(sim_pose::Phase::Drive(0.0), Some(SimFault::ShortCompression));
        let g = body.guide(Phase::Recovery, &fig).unwrap();
        assert!(g.you.legs > 0.15, "the seat stopped at {:.2} of its travel", g.you.legs);
        let (mine, catch) = (pt(&fig, Joint::Wrist).unwrap().0, pt(&body.ends().0, Joint::Wrist).unwrap().0);
        assert!(catch - mine > 0.2, "the handle should come {:.2} further", catch - mine);
    }

    #[test]
    fn the_handle_turning_round_changes_the_phase() {
        let mut d = Direction::default();
        // Coming up to the catch, small wobbles don't count.
        for x in [0.0, 0.3, 0.6, 0.55, 0.7] {
            assert_eq!(d.push(x).phase, Phase::Recovery);
        }
        let m = d.push(0.6);
        assert!(m.phase == Phase::Recovery && !m.furthest && !m.turned, "not far enough back to be sure");
        let m = d.push(0.45);
        assert!(m.phase == Phase::Drive && m.turned, "{m:?}");
        let m = d.push(-1.5);
        assert!(m.phase == Phase::Drive && m.furthest && !m.turned, "{m:?}");
        assert_eq!(d.push(-1.4).phase, Phase::Drive);
        assert_eq!(d.push(-1.2).phase, Phase::Recovery);
    }

    #[test]
    fn a_stroke_s_order_reads_off_the_handle() {
        let ideal = sim_body().sequence();
        let clean = sequence(None);
        let (d, r) = (clean.drive, clean.recovery);
        // Drive: legs first, the body after, the arms last.
        let (legs, body, arms) = (d.legs.unwrap(), d.body.unwrap(), d.arms.unwrap());
        assert!(legs.start < 0.1 && legs.start < body.start && body.start < arms.start, "{d:?}");
        assert!(legs.end < arms.end);
        // Recovery: arms, body, then the slide.
        let (legs, body, arms) = (r.legs.unwrap(), r.body.unwrap(), r.arms.unwrap());
        assert!(arms.start < body.start && body.start < legs.start, "{r:?}");
        // The clean rower is close to the target's order.
        for (name, mine, target) in [
            ("drive body", d.body, ideal.drive.body),
            ("drive arms", d.arms, ideal.drive.arms),
            ("recovery legs", r.legs, ideal.recovery.legs),
        ] {
            let (mine, target) = (mine.unwrap(), target.unwrap());
            assert!((mine.start - target.start).abs() < 0.12, "{name}: starts at {:.2}, target {:.2}", mine.start, target.start);
        }
        // Arms bent from the catch start well before the target's.
        let early = sequence(Some(SimFault::EarlyArms)).drive.arms.unwrap();
        assert!(early.start < ideal.drive.arms.unwrap().start - 0.25, "{early:?}");
    }

    #[test]
    fn a_part_that_hardly_moves_has_no_span() {
        // Legs only: the seat travels, the body and arms hold still.
        let legs_only: Vec<Sample> = (0..=20)
            .map(|i| Sample {
                handle: 0.8 - 1.1 * i as f64 / 20.0,
                seat: -0.9 - 1.1 * i as f64 / 20.0,
                lean: 25.0,
                elbow: 172.0,
            })
            .collect();
        let s = spans(&legs_only);
        assert!(s.legs.is_some() && s.body.is_none() && s.arms.is_none(), "{s:?}");
        // The handle barely moving isn't a stroke.
        assert_eq!(spans(&legs_only[..5]), Spans::default());
        assert_eq!(spans(&[]), Spans::default());
    }
}
