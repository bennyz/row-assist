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

/// Knee from hip and ankle (two-link IK), on the upper side.
fn knee(hip: (f64, f64)) -> (f64, f64) {
    let (dx, dy) = (ANKLE.0 - hip.0, ANKLE.1 - hip.1);
    let d = dx.hypot(dy).min(THIGH + SHIN - 1e-6);
    let a = (THIGH * THIGH - SHIN * SHIN + d * d) / (2.0 * d);
    let h = (THIGH * THIGH - a * a).max(0.0).sqrt();
    let (mx, my) = (hip.0 + a * dx / d, hip.1 + a * dy / d);
    let (k1, k2) = ((mx - h * dy / d, my + h * dx / d), (mx + h * dy / d, my - h * dx / d));
    if k1.1 >= k2.1 { k1 } else { k2 }
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
                _ => (smooth(d / 0.6), smooth((d - 0.25) / 0.55), smooth((d - 0.55) / 0.45)),
            };
            // Shooting the slide: hips run away and the trunk tips further forward.
            let lean = if fault == Some(ShootingSlide) { 12.0 * (PI * (d / 0.45).clamp(0.0, 1.0)).sin() } else { 0.0 };
            (legs, body, arms, lean)
        }
        Phase::Recovery(r) => (
            1.0 - smooth((r - 0.3) / 0.7),
            1.0 - smooth((r - 0.1) / 0.3),
            1.0 - smooth(r / 0.2),
            0.0,
        ),
    };
    let catch_x = catch_hip_x() - if fault == Some(ShortCompression) { 60.0 } else { 0.0 };
    let hip = (catch_x + (finish_hip_x() - catch_x) * legs, HIP_Y);
    let kn = knee(hip);
    let trunk = (LEAN + lean_extra - 2.0 * LEAN * body).to_radians();
    let shoulder = (hip.0 + TORSO * trunk.sin(), hip.1 + TORSO * trunk.cos());
    let ear = (shoulder.0 + NECK * trunk.sin(), shoulder.1 + NECK * trunk.cos());
    // Arm angles from straight down, forward positive: hands near the feet at
    // the catch, elbows past the body at the finish.
    let upper = (20.0 - 15.0 * body - 35.0 * arms).to_radians();
    let elbow_deg = 180.0 - 105.0 * arms;
    let fore = upper + (180.0 - elbow_deg).to_radians();
    let elbow = (shoulder.0 + UPPER_ARM * upper.sin(), shoulder.1 - UPPER_ARM * upper.cos());
    let wrist = (elbow.0 + FOREARM * fore.sin(), elbow.1 - FOREARM * fore.cos());
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
                for fault in [None, Some(SimFault::EarlyArms), Some(SimFault::ShootingSlide), Some(SimFault::ShortCompression)] {
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
    fn short_compression_keeps_the_knee_behind_the_ankle() {
        let c = figure(Phase::Drive(0.0), Some(SimFault::ShortCompression));
        assert!(c[KNEE].0 < c[ANK].0 - 20.0);
    }
}
