//! Technique per stroke: the camera's view of each stroke lined up with the
//! PM5's, and the faults it shows (DESIGN.md §5.1, faults 1–4).
//!
//! Thresholds start from common coaching norms and are to be tuned on
//! deliberately-rowed recordings.

use model::{Event, EventKind, Pose};
use serde::{Deserialize, Serialize};

use crate::pose::{Angles, Joint, Side, angles, camera_tilt, has_pose, joint, near_side, thigh_px};
use crate::strokes::StrokeRecord;

/// Strokes before this are settling in; calibration skips them.
const SYNC_SKIP: usize = 3;
const SYNC_STROKES: usize = 20;
const SYNC_MIN: usize = 5;
/// Window around the PM5 catch to look for the camera catch.
const SYNC_BEFORE_US: i64 = 600_000;
const SYNC_AFTER_US: i64 = 300_000;
/// After alignment, the camera catch is looked for this close to the PM5's.
const CATCH_WINDOW_US: i64 = 300_000;

pub const SHORT_COMPRESSION_SHIN_DEG: f64 = 12.0;
pub const SHOOTING_TRUNK_GAIN_DEG: f64 = 6.0;
pub const EARLY_ARMS_ELBOW_DEG: f64 = 160.0;
/// Below this knee angle the legs are still driving.
pub const LEGS_DOWN_KNEE_DEG: f64 = 150.0;
pub const RUSHED_RATIO: f64 = 1.5;
pub const RUSHED_MAX_SPM: f64 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fault {
    /// Doesn't reach full compression: shins short of vertical at the catch.
    ShortCompression,
    /// Seat moves before the handle: the trunk tips forward early in the drive.
    ShootingSlide,
    /// Arms bend while the legs are still driving.
    EarlyArms,
    /// Recovery under 1.5× the drive at a steady rate.
    RushedRecovery,
}

impl Fault {
    pub const ALL: [Fault; 4] = [Fault::ShortCompression, Fault::ShootingSlide, Fault::EarlyArms, Fault::RushedRecovery];

    pub fn label(self) -> &'static str {
        match self {
            Self::ShortCompression => "short compression",
            Self::ShootingSlide => "shooting the slide",
            Self::EarlyArms => "early arms",
            Self::RushedRecovery => "rushed recovery",
        }
    }

    /// What a coach would say (≤ 4 words).
    pub fn cue(self) -> &'static str {
        match self {
            Self::ShortCompression => "Get to full compression",
            Self::ShootingSlide => "Handle moves with the seat",
            Self::EarlyArms => "Legs first, arms long",
            Self::RushedRecovery => "Slow the slide",
        }
    }

    /// Needs the camera (the others come from the PM5 alone).
    pub fn needs_camera(self) -> bool {
        !matches!(self, Self::RushedRecovery)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FaultHit {
    pub fault: Fault,
    /// The measurement that tripped it (degrees, or a ratio).
    pub value: f64,
}

/// Near-side joints in `POSE_JOINTS` order, in thigh lengths from the hip,
/// x toward the feet, y up. `None` for a joint not seen (not NaN: JSON has
/// no NaN, and these are stored).
pub type Figure = Vec<Option<[f32; 2]>>;

/// How the camera saw the session.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraView {
    pub side: Side,
    pub tilt_deg: f64,
    /// PM5 drive start minus camera catch, ms (camera latency, clock offset
    /// and the PM5's own detection delay, together). `None`: not enough strokes.
    pub offset_ms: Option<f64>,
    pub frames: usize,
    /// Thigh length in pixels: figures are stored in thighs, so camera distance doesn't matter.
    pub thigh_px: f64,
    /// +1 when the feet are to the right in the image.
    pub facing: f64,
}

/// The camera's view of one stroke.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StrokePose {
    pub frames: u32,
    /// At the camera catch (wrist furthest forward) and finish (furthest back).
    pub catch: Angles,
    pub finish: Angles,
    /// Camera catch and finish, ms after the PM5 catch.
    pub catch_ms: f64,
    pub finish_ms: f64,
    /// Most the trunk tipped forward past its catch angle in the first third of the drive.
    pub early_trunk_gain: Option<f64>,
    /// Smallest elbow angle while the knee was still under 150°.
    pub elbow_while_legs_bent: Option<f64>,
    /// The body at catch and finish, levelled.
    pub catch_figure: Figure,
    pub finish_figure: Figure,
}

/// Pose frames from a recording, by capture time.
pub fn pose_frames<'a>(events: impl IntoIterator<Item = &'a Event>) -> Vec<Pose> {
    let mut frames: Vec<Pose> = events
        .into_iter()
        .filter_map(|e| match &e.kind {
            EventKind::Pose(p) if has_pose(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    frames.sort_by_key(|p| p.captured_us);
    frames
}

/// Levelled joint, or `None` if not seen.
fn at(f: &Pose, view: &CameraView, j: Joint) -> Option<(f64, f64)> {
    let p = joint(f, view.side, j).filter(|p| p.visibility >= crate::pose::VISIBLE)?;
    let (s, c) = (-view.tilt_deg).to_radians().sin_cos();
    Some((p.x * c - p.y * s, p.x * s + p.y * c))
}

/// How far forward (toward the feet) the wrist is, in pixels.
fn reach(f: &Pose, view: &CameraView) -> Option<f64> {
    at(f, view, Joint::Wrist).map(|(x, _)| x * view.facing)
}

pub fn camera_view(frames: &[Pose]) -> Option<CameraView> {
    let side = near_side(frames)?;
    let tilt_deg = camera_tilt(frames, side).unwrap_or(0.0);
    let thigh = thigh_px(frames, side)?;
    let mut dirs: Vec<f64> = frames
        .iter()
        .filter_map(|f| Some((joint(f, side, Joint::Ankle)?.x - joint(f, side, Joint::Hip)?.x).signum()))
        .collect();
    dirs.sort_by(f64::total_cmp);
    let facing = *dirs.get(dirs.len() / 2)?;
    Some(CameraView {
        side,
        tilt_deg,
        offset_ms: None,
        frames: frames.len(),
        thigh_px: thigh,
        facing,
    })
}

/// Index of the first frame captured at or after `t_us`.
fn first_at(frames: &[Pose], t_us: i64) -> usize {
    frames.partition_point(|f| f.captured_us < t_us)
}

/// Capture time of the frame with the wrist furthest forward among
/// `frames[lo..hi]`. No sub-frame fitting: the catch is lopsided (the drive
/// leaves it several times faster than the recovery arrives), which pulls a
/// fitted peak toward the recovery side by up to half a frame. The best frame
/// is off by at most half a frame, with no bias.
fn reach_peak(frames: &[Pose], lo: usize, hi: usize, view: &CameraView) -> Option<f64> {
    let (i, _) = (lo..hi)
        .filter_map(|i| reach(&frames[i], view).map(|r| (i, r)))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    Some(frames[i].captured_us as f64)
}

/// Median of PM5 catch minus camera catch over the early strokes, in ms.
pub fn sync_offset(strokes: &[StrokeRecord], frames: &[Pose], view: &CameraView) -> Option<f64> {
    let mut offsets: Vec<f64> = strokes
        .iter()
        .skip(SYNC_SKIP)
        .take(SYNC_STROKES)
        .filter_map(|s| {
            let lo = first_at(frames, s.t_catch_us - SYNC_BEFORE_US);
            let hi = first_at(frames, s.t_catch_us + SYNC_AFTER_US);
            let t = reach_peak(frames, lo, hi, view)?;
            Some((s.t_catch_us as f64 - t) / 1000.0)
        })
        .collect();
    if offsets.len() < SYNC_MIN {
        return None;
    }
    offsets.sort_by(f64::total_cmp);
    Some(offsets[offsets.len() / 2])
}

/// Joints relative to the hip, in thigh lengths, x toward the feet.
fn figure(f: &Pose, view: &CameraView) -> Figure {
    let Some(hip) = at(f, view, Joint::Hip) else {
        return Vec::new();
    };
    use Joint::*;
    [Ear, Shoulder, Elbow, Wrist, Hip, Knee, Ankle, Heel, Toe]
        .iter()
        .map(|&j| {
            at(f, view, j).map(|(x, y)| {
                [
                    ((x - hip.0) * view.facing / view.thigh_px) as f32,
                    ((y - hip.1) / view.thigh_px) as f32,
                ]
            })
        })
        .collect()
}

/// The camera's view of one stroke, with frames shifted by the sync offset.
pub fn stroke_pose(stroke: &StrokeRecord, frames: &[Pose], view: &CameraView) -> Option<StrokePose> {
    let offset_us = (view.offset_ms.unwrap_or(0.0) * 1000.0) as i64;
    // Camera time = PM5 time − offset.
    let to_cam = |t: i64| t - offset_us;
    let end = stroke.t_next_catch_us.unwrap_or(stroke.t_catch_us + 4_000_000);
    let lo = first_at(frames, to_cam(stroke.t_catch_us - CATCH_WINDOW_US));
    let hi = first_at(frames, to_cam(end - CATCH_WINDOW_US / 2));
    let catch_hi = first_at(frames, to_cam(stroke.t_catch_us + CATCH_WINDOW_US));
    if hi <= lo {
        return None;
    }
    let catch_i = (lo..catch_hi.min(hi))
        .filter_map(|i| reach(&frames[i], view).map(|r| (i, r)))
        .max_by(|a, b| a.1.total_cmp(&b.1))?
        .0;
    let finish_i = (catch_i..hi)
        .filter_map(|i| reach(&frames[i], view).map(|r| (i, r)))
        .min_by(|a, b| a.1.total_cmp(&b.1))?
        .0;
    let ang = |i: usize| angles(&frames[i], view.side, view.tilt_deg);
    let ms = |i: usize| (frames[i].captured_us + offset_us - stroke.t_catch_us) as f64 / 1000.0;

    let catch = ang(catch_i);
    // First third of the drive, by the PM5's drive time.
    let third_us = (stroke.drive_ms.unwrap_or(1000.0) * 1000.0 / 3.0) as i64;
    let third = first_at(frames, frames[catch_i].captured_us + third_us).max(catch_i + 1).min(hi);
    let early_trunk_gain = catch
        .trunk
        .and_then(|t0| (catch_i..third).filter_map(|i| ang(i).trunk).map(|t| t - t0).reduce(f64::max));
    let elbow_while_legs_bent = (catch_i..=finish_i)
        .map(ang)
        .filter(|a| a.knee.is_some_and(|k| k < LEGS_DOWN_KNEE_DEG))
        .filter_map(|a| a.elbow)
        .reduce(f64::min);

    Some(StrokePose {
        frames: (hi - lo) as u32,
        catch,
        finish: ang(finish_i),
        catch_ms: ms(catch_i),
        finish_ms: ms(finish_i),
        early_trunk_gain,
        elbow_while_legs_bent,
        catch_figure: figure(&frames[catch_i], view),
        finish_figure: figure(&frames[finish_i], view),
    })
}

/// Faults in one stroke. Camera faults need `pose`.
pub fn faults(stroke: &StrokeRecord, pose: Option<&StrokePose>) -> Vec<FaultHit> {
    let mut hits = Vec::new();
    let mut hit = |fault, value| hits.push(FaultHit { fault, value });
    if let Some(p) = pose {
        if let Some(shin) = p.catch.shin.filter(|&s| s > SHORT_COMPRESSION_SHIN_DEG) {
            hit(Fault::ShortCompression, shin);
        }
        if let Some(gain) = p.early_trunk_gain.filter(|&g| g > SHOOTING_TRUNK_GAIN_DEG) {
            hit(Fault::ShootingSlide, gain);
        }
        if let Some(elbow) = p.elbow_while_legs_bent.filter(|&e| e < EARLY_ARMS_ELBOW_DEG) {
            hit(Fault::EarlyArms, elbow);
        }
    }
    if let (Some(ratio), Some(rate)) = (stroke.ratio, stroke.rate())
        && rate <= RUSHED_MAX_SPM
        && ratio < RUSHED_RATIO
    {
        hit(Fault::RushedRecovery, ratio);
    }
    hits
}

/// How often each fault showed up.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FaultRate {
    pub fault: Fault,
    pub hits: u32,
    /// Strokes it could be judged on.
    pub of: u32,
}

pub fn fault_rates(strokes: &[StrokeRecord]) -> Vec<FaultRate> {
    Fault::ALL
        .iter()
        .map(|&fault| {
            let judged = strokes.iter().filter(|s| {
                s.t_next_catch_us.is_some() && (!fault.needs_camera() || s.pose.is_some())
            });
            let (mut hits, mut of) = (0, 0);
            for s in judged {
                of += 1;
                hits += u32::from(s.faults.iter().any(|h| h.fault == fault));
            }
            FaultRate { fault, hits, of }
        })
        .filter(|r| r.of > 0)
        .collect()
}

/// The session's typical catch and finish: the median of each joint over
/// strokes the camera saw, the ghost the review screen draws behind a stroke.
pub fn typical_figures(strokes: &[StrokeRecord]) -> (Figure, Figure) {
    let median_figure = |pick: fn(&StrokePose) -> &Figure| -> Figure {
        let figures: Vec<&Figure> = strokes.iter().filter_map(|s| s.pose.as_ref()).map(pick).collect();
        let joints = figures.iter().map(|f| f.len()).max().unwrap_or(0);
        (0..joints)
            .map(|j| {
                let pts: Vec<[f32; 2]> = figures.iter().filter_map(|f| f.get(j).copied().flatten()).collect();
                (pts.len() * 2 >= figures.len()).then(|| {
                    let med = |k: usize| {
                        let mut v: Vec<f32> = pts.iter().map(|p| p[k]).collect();
                        v.sort_by(f32::total_cmp);
                        v[v.len() / 2]
                    };
                    [med(0), med(1)]
                })
            })
            .collect()
    };
    (median_figure(|p| &p.catch_figure), median_figure(|p| &p.finish_figure))
}

/// Attach pose and faults to every stroke. Returns how the camera saw the session.
pub fn apply(strokes: &mut [StrokeRecord], events: &[Event]) -> Option<CameraView> {
    let frames = pose_frames(events);
    let view = camera_view(&frames).map(|mut v| {
        v.offset_ms = sync_offset(strokes, &frames, &v);
        v
    });
    for s in strokes.iter_mut() {
        s.pose = view.as_ref().and_then(|v| stroke_pose(s, &frames, v));
        s.faults = faults(s, s.pose.as_ref());
    }
    view
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use devices::sim::{SimConfig, generate};
    use devices::sim_pose::{PoseSim, SimFault};

    use super::*;
    use crate::session::tests::T0;

    /// Two minutes rowed by the kinematic rower.
    fn row(fault: Option<SimFault>, drive_fraction: f64) -> (Vec<StrokeRecord>, CameraView) {
        let cfg = SimConfig {
            drive_fraction,
            pose: Some(PoseSim {
                fault,
                ..Default::default()
            }),
            ..Default::default()
        };
        let events = generate(cfg, T0, Duration::from_secs(120));
        let a = crate::analyze(&events, None);
        let view = a.summary.unwrap().camera.expect("camera view");
        (a.strokes, view)
    }

    fn rate(strokes: &[StrokeRecord], fault: Fault) -> f64 {
        let r = fault_rates(strokes).into_iter().find(|r| r.fault == fault).unwrap();
        r.hits as f64 / r.of as f64
    }

    #[test]
    fn sync_finds_the_pm5_lag() {
        let (strokes, view) = row(None, 0.33);
        let offset = view.offset_ms.unwrap();
        // At 30 fps the catch frame is within half a frame (17 ms) of the true catch.
        assert!((offset - 60.0).abs() <= 17.0, "offset {offset} ms, simulated 60");
        assert_eq!(view.side, Side::Left);
        assert_eq!(view.facing, 1.0);
        assert!(view.tilt_deg.abs() < 0.5, "level camera: {}", view.tilt_deg);
        // Once aligned, the camera catch lands on the PM5's.
        let p = strokes[10].pose.as_ref().unwrap();
        assert!(p.catch_ms.abs() <= 34.0, "within a frame: {}", p.catch_ms);
    }

    #[test]
    fn clean_rowing_has_no_faults() {
        let (strokes, _) = row(None, 0.33);
        for fault in Fault::ALL {
            assert_eq!(rate(&strokes, fault), 0.0, "{fault:?}");
        }
        let p = strokes[10].pose.as_ref().unwrap();
        assert!(p.catch.shin.unwrap().abs() < 3.0, "vertical shins: {:?}", p.catch.shin);
        assert!(p.catch.trunk.unwrap() > 15.0 && p.finish.trunk.unwrap() < -15.0);
        // Figures: hip at the origin, the knee over the ankle at the catch.
        let f = &p.catch_figure;
        assert_eq!(f[4], Some([0.0, 0.0]));
        let (knee, ankle) = (f[5].unwrap(), f[6].unwrap());
        assert!((knee[0] - ankle[0]).abs() < 0.05, "knee {knee:?} ankle {ankle:?}");
        let (catch, finish) = typical_figures(&strokes);
        assert_eq!((catch.len(), finish.len()), (9, 9));
        assert!(finish[3].unwrap()[0] < catch[3].unwrap()[0] - 1.0, "the handle travels");
    }

    #[test]
    fn each_simulated_fault_is_found_and_only_that_one() {
        for (sim, fault) in [
            (SimFault::ShortCompression, Fault::ShortCompression),
            (SimFault::ShootingSlide, Fault::ShootingSlide),
            (SimFault::EarlyArms, Fault::EarlyArms),
        ] {
            let (strokes, _) = row(Some(sim), 0.33);
            assert!(rate(&strokes, fault) > 0.9, "{fault:?}: {}", rate(&strokes, fault));
            for other in Fault::ALL.into_iter().filter(|&f| f != fault) {
                assert!(rate(&strokes, other) < 0.1, "{sim:?} also tripped {other:?}: {}", rate(&strokes, other));
            }
        }
    }

    #[test]
    fn records_with_pose_survive_json() {
        let (mut strokes, _) = row(Some(SimFault::EarlyArms), 0.33);
        // A joint the camera lost must still store and load.
        strokes[5].pose.as_mut().unwrap().catch_figure[3] = None;
        let json = serde_json::to_string(&strokes).unwrap();
        let back: Vec<StrokeRecord> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), strokes.len());
        assert_eq!(back[5].pose.as_ref().unwrap().catch_figure[3], None);
        assert_eq!(back[5].faults, strokes[5].faults);
    }

    #[test]
    fn rushed_recovery_comes_from_timing_alone() {
        // Drive 45% of the cycle: recovery 1.2× the drive.
        let (strokes, _) = row(None, 0.45);
        assert!(rate(&strokes, Fault::RushedRecovery) > 0.9);
        let no_camera: Vec<StrokeRecord> = strokes
            .iter()
            .cloned()
            .map(|mut s| {
                s.pose = None;
                s.faults = faults(&s, None);
                s
            })
            .collect();
        assert!(rate(&no_camera, Fault::RushedRecovery) > 0.9);
        assert!(fault_rates(&no_camera).iter().all(|r| !r.fault.needs_camera()));
    }
}
