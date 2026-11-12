//! A kinematic rower for the simulator: a side-on stick figure moved through
//! the stroke in the coached order (drive: legs, body, arms; recovery: arms,
//! body, slide), with faults that can be switched on. Pose analysis can then
//! be built and tested against known answers before real recordings exist.
//!
//! The figure faces right (feet toward the flywheel), left side to the camera,
//! in a 1280×720 frame.

use std::f64::consts::PI;

use model::Pose;

/// A technique fault the simulated rower can make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimFault {
    /// Arms bend from the start of the drive instead of after the legs.
    EarlyArms,
    /// Seat moves early while the body leans further forward: the handle lags.
    ShootingSlide,
    /// Doesn't come all the way forward: shins short of vertical at the catch.
    ShortCompression,
    /// The body swings back from the catch, together with the legs.
    EarlyBody,
    /// On the way forward the knees come up at once, with the hands.
    EarlyKnees,
}

#[derive(Debug, Clone)]
pub struct PoseSim {
    pub fps: f64,
    /// How long after the real catch the PM5 reports the drive. Sync
    /// calibration should find this.
    pub pm5_lag_ms: f64,
    pub fault: Option<SimFault>,
}

impl Default for PoseSim {
    fn default() -> Self {
        Self {
            fps: 30.0,
            pm5_lag_ms: 60.0,
            fault: None,
        }
    }
}

pub const WIDTH: u16 = 1280;
pub const HEIGHT: u16 = 720;

// Body in pixels.
const THIGH: f64 = 175.0;
const SHIN: f64 = 175.0;
const TORSO: f64 = 230.0;
const UPPER_ARM: f64 = 125.0;
const FOREARM: f64 = 115.0;
const NECK: f64 = 55.0;
// Erg, y up from the bottom of the frame.
const ANKLE: (f64, f64) = (980.0, 170.0);
const HIP_Y: f64 = 210.0;
/// Trunk lean at catch (forward) and finish (back), degrees.
const LEAN: f64 = 25.0;
/// Height the handle travels at with straight arms (the chain's line): half
/// a thigh above the hip, as on a real row.
const HANDLE_Y: f64 = 300.0;

/// Where the rower is in the stroke.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    /// 0 at the catch to 1 at the finish.
    Drive(f64),
    /// 0 at the finish to 1 at the next catch.
    Recovery(f64),
}

fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Seat position when the knee is right above the ankle.
fn catch_hip_x() -> f64 {
    let knee_y = ANKLE.1 + SHIN;
    ANKLE.0 - (THIGH * THIGH - (knee_y - HIP_Y).powi(2)).sqrt()
}

/// Seat position with the legs (almost) straight.
fn finish_hip_x() -> f64 {
    let reach = THIGH + SHIN - 1.0;
    ANKLE.0 - (reach * reach - (ANKLE.1 - HIP_Y).powi(2)).sqrt()
}

/// The middle joint of a two-link chain from `base` to `tip` (lengths `l1`,
/// `l2`): above the base–tip line if `up`, else below.
fn middle(base: (f64, f64), tip: (f64, f64), l1: f64, l2: f64, up: bool) -> (f64, f64) {
    let (dx, dy) = (tip.0 - base.0, tip.1 - base.1);
    let d = dx.hypot(dy).clamp(1e-6, l1 + l2 - 1e-6);
    let a = (l1 * l1 - l2 * l2 + d * d) / (2.0 * d);
    let h = (l1 * l1 - a * a).max(0.0).sqrt();
    let (mx, my) = (base.0 + a * dx / d, base.1 + a * dy / d);
    let (p1, p2) = ((mx - h * dy / d, my + h * dx / d), (mx + h * dy / d, my - h * dx / d));
    if (p1.1 >= p2.1) == up { p1 } else { p2 }
}

fn lerp(a: (f64, f64), b: (f64, f64), t: f64) -> (f64, f64) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// Joint positions for `POSE_JOINTS` (ear … toe), pixels, y down.
pub fn figure(phase: Phase, fault: Option<SimFault>) -> [(f64, f64); 9] {
    use SimFault::*;
    // Progress of each part from its catch position (0) to its finish position (1).
    let (legs, body, arms, lean_extra) = match phase {
        Phase::Drive(d) => {
            let (legs, body, arms) = match fault {
                Some(EarlyArms) => (smooth(d / 0.6), smooth((d - 0.25) / 0.55), smooth((d - 0.05) / 0.6)),
                Some(ShootingSlide) => (smooth(d / 0.4), smooth((d - 0.45) / 0.45), smooth((d - 0.6) / 0.4)),
                Some(EarlyBody) => (smooth(d / 0.6), smooth(d / 0.7), smooth((d - 0.55) / 0.45)),
                _ => (smooth(d / 0.6), smooth((d - 0.25) / 0.55), smooth((d - 0.55) / 0.45)),
            };
            // Shooting the slide: hips run away and the trunk tips further forward.
            let lean = if fault == Some(ShootingSlide) { 12.0 * (PI * (d / 0.45).clamp(0.0, 1.0)).sin() } else { 0.0 };
            (legs, body, arms, lean)
        }
        Phase::Recovery(r) => (
            1.0 - smooth(if fault == Some(EarlyKnees) { r } else { (r - 0.3) / 0.7 }),
            1.0 - smooth((r - 0.1) / 0.3),
            1.0 - smooth(r / 0.2),
            0.0,
        ),
    };
    let catch_x = catch_hip_x() - if fault == Some(ShortCompression) { 60.0 } else { 0.0 };
    let hip = (catch_x + (finish_hip_x() - catch_x) * legs, HIP_Y);
    let kn = middle(hip, ANKLE, THIGH, SHIN, true);
    let trunk = (LEAN + lean_extra - 2.0 * LEAN * body).to_radians();
    let shoulder = (hip.0 + TORSO * trunk.sin(), hip.1 + TORSO * trunk.cos());
    let ear = (shoulder.0 + NECK * trunk.sin(), shoulder.1 + NECK * trunk.cos());
    // Hands follow the handle: straight arms reach along the chain's line;
    // drawing in brings them to the lower ribs. The chain pulls the handle
    // toward the flywheel, so it only ever moves forward in the recovery.
    let arm = UPPER_ARM + FOREARM - 1.0;
    let drop = shoulder.1 - HANDLE_Y;
    let straight = if drop.abs() < arm {
        (shoulder.0 + (arm * arm - drop * drop).sqrt(), HANDLE_Y)
    } else {
        (shoulder.0, shoulder.1 - arm)
    };
    let (ts, tc) = trunk.sin_cos();
    let ribs = (shoulder.0 - 0.45 * TORSO * ts + 60.0 * tc, shoulder.1 - 0.45 * TORSO * tc - 60.0 * ts);
    let wrist = lerp(straight, ribs, arms);
    let elbow = middle(shoulder, wrist, UPPER_ARM, FOREARM, false);
    let heel = (ANKLE.0 - 15.0, ANKLE.1 - 20.0);
    let toe = (ANKLE.0 + 70.0, ANKLE.1 - 25.0);
    [ear, shoulder, elbow, wrist, hip, kn, ANKLE, heel, toe].map(|(x, y)| (x, HEIGHT as f64 - y))
}

/// A frame as the browser would send it: near (left) side clear, far side
/// a few pixels behind and less certain, with `noise` px of jitter.
pub fn pose_frame(captured_us: i64, joints: [(f64, f64); 9], mut noise: impl FnMut() -> f64) -> Pose {
    let (w, h) = (WIDTH as f64, HEIGHT as f64);
    let mut side = |dx: f64, v: f32| -> Vec<[f32; 3]> {
        joints
            .iter()
            .map(|&(x, y)| [((x + dx + noise()) / w) as f32, ((y + noise()) / h) as f32, v])
            .collect()
    };
    let near = side(0.0, 0.95);
    let far = side(8.0, 0.6);
    Pose {
        captured_us,
        width: WIDTH,
        height: HEIGHT,
        points: [near, far].concat(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOULDER: usize = 1;
    const ELBOW: usize = 2;
    const WRIST: usize = 3;
    const HIP: usize = 4;
    const KNEE: usize = 5;
    const ANK: usize = 6;

    fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
        (a.0 - b.0).hypot(a.1 - b.1)
    }

    #[test]
    fn segments_keep_their_length_through_the_stroke() {
        for i in 0..=20 {
            let p = i as f64 / 20.0;
            for phase in [Phase::Drive(p), Phase::Recovery(p)] {
                for fault in [
                    None,
                    Some(SimFault::EarlyArms),
                    Some(SimFault::ShootingSlide),
                    Some(SimFault::ShortCompression),
                    Some(SimFault::EarlyBody),
                    Some(SimFault::EarlyKnees),
                ] {
                    let j = figure(phase, fault);
                    assert!((dist(j[HIP], j[KNEE]) - THIGH).abs() < 1e-6);
                    assert!((dist(j[KNEE], j[ANK]) - SHIN).abs() < 1e-6);
                    assert!((dist(j[SHOULDER], j[ELBOW]) - UPPER_ARM).abs() < 1e-6);
                    assert!((dist(j[ELBOW], j[WRIST]) - FOREARM).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn catch_and_finish_look_right() {
        let c = figure(Phase::Drive(0.0), None);
        assert!((c[KNEE].0 - c[ANK].0).abs() < 1e-6, "vertical shin at the catch");
        assert!(c[SHOULDER].0 > c[HIP].0, "leaning toward the feet");
        let f = figure(Phase::Drive(1.0), None);
        assert!(f[SHOULDER].0 < f[HIP].0, "leaning back at the finish");
        assert!(f[WRIST].0 < c[WRIST].0 - 300.0, "handle travels");
        // The stroke closes: end of recovery is the catch again.
        let back = figure(Phase::Recovery(1.0), None);
        for (a, b) in back.iter().zip(&c) {
            assert!(dist(*a, *b) < 1e-6);
        }
    }

    #[test]
    fn the_handle_goes_back_in_the_drive_and_forward_in_the_recovery() {
        let wrist_x = |phase| figure(phase, None)[WRIST].0;
        let steps = |f: fn(f64) -> Phase| (0..=50).map(move |i| wrist_x(f(i as f64 / 50.0))).collect::<Vec<_>>();
        let drive = steps(Phase::Drive);
        let recovery = steps(Phase::Recovery);
        assert!(drive.windows(2).all(|w| w[1] <= w[0] + 1e-9), "drive: {drive:?}");
        assert!(recovery.windows(2).all(|w| w[1] >= w[0] - 1e-9), "recovery: {recovery:?}");
    }

    #[test]
    fn short_compression_keeps_the_knee_behind_the_ankle() {
        let c = figure(Phase::Drive(0.0), Some(SimFault::ShortCompression));
        assert!(c[KNEE].0 < c[ANK].0 - 20.0);
    }
}
