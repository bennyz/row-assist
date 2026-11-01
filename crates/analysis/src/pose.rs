//! Body position from camera landmarks: which side faces the camera, joint
//! angles, camera tilt, and whether the camera is set up well enough to judge.
//!
//! Points are in pixels with y pointing up (flipped from the image), so
//! angles read the way a coach would describe them.

use model::{POSE_JOINTS, Pose};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// Index into [`POSE_JOINTS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joint {
    Ear,
    Shoulder,
    Elbow,
    Wrist,
    Hip,
    Knee,
    Ankle,
    Heel,
    Toe,
}

use Joint::*;

impl Joint {
    pub fn name(self) -> &'static str {
        POSE_JOINTS[self as usize]
    }
}

const PER_SIDE: usize = POSE_JOINTS.len();
/// The joints every angle needs; the framing check wants all of them in view.
pub const CORE: [Joint; 6] = [Shoulder, Elbow, Wrist, Hip, Knee, Ankle];
/// A landmark counts as seen at this visibility.
pub const VISIBLE: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    /// Up is positive.
    pub y: f64,
    pub visibility: f64,
}

/// A joint in pixels, if the frame has a pose.
pub fn joint(pose: &Pose, side: Side, j: Joint) -> Option<Point> {
    let i = j as usize + if side == Side::Right { PER_SIDE } else { 0 };
    let [x, y, v] = *pose.points.get(i)?;
    Some(Point {
        x: x as f64 * pose.width as f64,
        y: -(y as f64) * pose.height as f64,
        visibility: v as f64,
    })
}

fn seen(pose: &Pose, side: Side, j: Joint) -> Option<Point> {
    joint(pose, side, j).filter(|p| p.visibility >= VISIBLE)
}

pub fn has_pose(pose: &Pose) -> bool {
    pose.points.len() == 2 * PER_SIDE
}

/// The side facing the camera: the one the model is surer of over these frames.
pub fn near_side<'a>(frames: impl IntoIterator<Item = &'a Pose>) -> Option<Side> {
    let (mut left, mut right) = (0.0, 0.0);
    for f in frames.into_iter().filter(|f| has_pose(f)) {
        for j in CORE {
            left += joint(f, Side::Left, j).map_or(0.0, |p| p.visibility);
            right += joint(f, Side::Right, j).map_or(0.0, |p| p.visibility);
        }
    }
    match left.total_cmp(&right) {
        std::cmp::Ordering::Greater => Some(Side::Left),
        std::cmp::Ordering::Less => Some(Side::Right),
        std::cmp::Ordering::Equal => None,
    }
}

/// Joint angles in degrees for one frame, `None` where a joint isn't seen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Angles {
    /// Hip–knee–ankle; 180 is a straight leg.
    pub knee: Option<f64>,
    /// Shoulder–hip–knee: how closed the body is onto the thighs.
    pub hip: Option<f64>,
    /// Shoulder–elbow–wrist; 180 is a straight arm.
    pub elbow: Option<f64>,
    /// Hip→shoulder from vertical; positive leans toward the feet (catch),
    /// negative leans back (finish).
    pub trunk: Option<f64>,
    /// Ankle→knee from vertical; 0 is a vertical shin (full compression),
    /// positive as the knee drops toward the hips.
    pub shin: Option<f64>,
}

fn angle_at(a: Point, b: Point, c: Point) -> f64 {
    let (ux, uy, vx, vy) = (a.x - b.x, a.y - b.y, c.x - b.x, c.y - b.y);
    (ux * vy - uy * vx).abs().atan2(ux * vx + uy * vy).to_degrees()
}

/// Rotate by `-tilt_deg` so a tilted camera's horizon becomes level.
fn level(p: Point, tilt_deg: f64) -> Point {
    let (s, c) = (-tilt_deg).to_radians().sin_cos();
    Point {
        x: p.x * c - p.y * s,
        y: p.x * s + p.y * c,
        ..p
    }
}

/// `tilt_deg` from [`camera_tilt`] (0 if unknown).
pub fn angles(pose: &Pose, side: Side, tilt_deg: f64) -> Angles {
    let j = |joint: Joint| seen(pose, side, joint).map(|p| level(p, tilt_deg));
    let (sh, el, wr, hip, kn, an) = (j(Shoulder), j(Elbow), j(Wrist), j(Hip), j(Knee), j(Ankle));
    // The feet are at the front of the erg: "forward" is from hip toward ankle.
    let forward = hip.zip(an).map(|(h, a)| (a.x - h.x).signum());
    let from_vertical = |from: Point, to: Point, dir: f64| (dir * (to.x - from.x)).atan2(to.y - from.y).to_degrees();
    Angles {
        knee: hip.zip(kn).zip(an).map(|((h, k), a)| angle_at(h, k, a)),
        hip: sh.zip(hip).zip(kn).map(|((s, h), k)| angle_at(s, h, k)),
        elbow: sh.zip(el).zip(wr).map(|((s, e), w)| angle_at(s, e, w)),
        trunk: hip.zip(sh).zip(forward).map(|((h, s), f)| from_vertical(h, s, f)),
        shin: an.zip(kn).zip(forward).map(|((a, k), f)| from_vertical(a, k, -f)),
    }
}

/// Median hip–knee length in pixels: the body's own ruler.
pub fn thigh_px<'a>(frames: impl IntoIterator<Item = &'a Pose>, side: Side) -> Option<f64> {
    let mut v: Vec<f64> = frames
        .into_iter()
        .filter_map(|f| {
            let (h, k) = (seen(f, side, Hip)?, seen(f, side, Knee)?);
            Some((h.x - k.x).hypot(h.y - k.y))
        })
        .collect();
    median(&mut v)
}

/// Camera roll in degrees, from the hip's path: the seat runs on a level rail,
/// so a sloping path means a tilted camera. Needs the hip to travel at least
/// half a thigh length (i.e. some rowing).
pub fn camera_tilt<'a>(frames: impl IntoIterator<Item = &'a Pose> + Clone, side: Side) -> Option<f64> {
    let thigh = thigh_px(frames.clone(), side)?;
    let pts: Vec<Point> = frames.into_iter().filter_map(|f| seen(f, side, Hip)).collect();
    let (lo, hi) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.x), hi.max(p.x)));
    if pts.len() < 10 || hi - lo < thigh / 2.0 {
        return None;
    }
    let n = pts.len() as f64;
    let (mx, my) = (pts.iter().map(|p| p.x).sum::<f64>() / n, pts.iter().map(|p| p.y).sum::<f64>() / n);
    let sxy: f64 = pts.iter().map(|p| (p.x - mx) * (p.y - my)).sum();
    let sxx: f64 = pts.iter().map(|p| (p.x - mx).powi(2)).sum();
    Some((sxy / sxx).atan().to_degrees())
}

fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[v.len() / 2])
}

/// One line of the setup checklist.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Check {
    pub key: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// Is the camera set up well enough to judge posture?
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Framing {
    pub frames: usize,
    pub fps: f64,
    pub side: Option<Side>,
    pub checks: Vec<Check>,
    pub ok: bool,
}

pub const MIN_FPS: f64 = 20.0;
/// Share of frames a condition must hold in.
const MOST: f64 = 0.8;
/// Far and near shoulders this far apart (in torso lengths) means the camera
/// sees the rower at an angle, not side-on.
pub const MAX_SHOULDER_SPREAD: f64 = 0.35;
/// Thigh shorter than this share of the frame height: too far away.
pub const MIN_THIGH_SHARE: f64 = 0.12;
pub const MIN_CLARITY: f64 = 0.75;
/// Landmarks this close to an edge count as out of frame.
const EDGE: f32 = 0.02;

/// Check the last second or so of frames (oldest first).
pub fn framing(frames: &[Pose]) -> Framing {
    let n = frames.len();
    let fps = match (frames.first(), frames.last()) {
        (Some(a), Some(b)) if n > 1 && b.captured_us > a.captured_us => {
            (n - 1) as f64 / ((b.captured_us - a.captured_us) as f64 / 1e6)
        }
        _ => 0.0,
    };
    let posed: Vec<&Pose> = frames.iter().filter(|f| has_pose(f)).collect();
    let side = near_side(posed.iter().copied());
    let mut checks = vec![Check {
        key: "camera",
        ok: fps >= MIN_FPS,
        detail: if n < 2 {
            "No frames yet".into()
        } else if fps >= MIN_FPS {
            format!("{fps:.0} fps")
        } else {
            format!("{fps:.0} fps, needs {MIN_FPS:.0}: close other apps, or use more light (cameras slow down in the dark)")
        },
    }];

    let person = n > 0 && posed.len() as f64 >= n as f64 * MOST;
    checks.push(Check {
        key: "person",
        ok: person,
        detail: if person {
            match side {
                Some(Side::Left) => "Left side to the camera".into(),
                Some(Side::Right) => "Right side to the camera".into(),
                None => "Can't tell which side faces the camera".into(),
            }
        } else {
            "No one in view: sit on the erg, side-on to the camera".into()
        },
    });
    let Some(side) = side.filter(|_| person) else {
        return finish(frames.len(), fps, None, checks);
    };

    let share = |test: &dyn Fn(&Pose) -> bool| posed.iter().filter(|f| test(f)).count() as f64 / posed.len() as f64;
    let in_frame = |f: &Pose, j: Joint| {
        let i = j as usize + if side == Side::Right { PER_SIDE } else { 0 };
        f.points.get(i).is_some_and(|&[x, y, v]| {
            v as f64 >= VISIBLE && (EDGE..=1.0 - EDGE).contains(&x) && (EDGE..=1.0 - EDGE).contains(&y)
        })
    };
    let missing: Vec<&str> = CORE
        .iter()
        .chain(&[Heel, Toe])
        .filter(|&&j| share(&|f| in_frame(f, j)) < MOST)
        .map(|j| j.name())
        .collect();
    checks.push(Check {
        key: "body",
        ok: missing.is_empty(),
        detail: if missing.is_empty() {
            "Whole body in view".into()
        } else {
            format!("Out of view: {}. Move the camera back, or check at both catch and finish", missing.join(", "))
        },
    });

    let mut spreads: Vec<f64> = posed
        .iter()
        .filter_map(|f| {
            let (near, far) = (joint(f, side, Shoulder)?, joint(f, side.other(), Shoulder)?);
            let hip = joint(f, side, Hip)?;
            let torso = (near.x - hip.x).hypot(near.y - hip.y);
            (torso > 0.0).then(|| (near.x - far.x).abs() / torso)
        })
        .collect();
    if let Some(spread) = median(&mut spreads) {
        let ok = spread <= MAX_SHOULDER_SPREAD;
        checks.push(Check {
            key: "side_on",
            ok,
            detail: if ok {
                "Side-on".into()
            } else {
                "Seen at an angle: put the camera square to the side of the erg, level with your hip".into()
            },
        });
    }

    let height = posed.first().map_or(1.0, |f| f.height.max(1) as f64);
    if let Some(thigh) = thigh_px(posed.iter().copied(), side) {
        let ok = thigh / height >= MIN_THIGH_SHARE;
        checks.push(Check {
            key: "size",
            ok,
            detail: if ok {
                format!("Thigh {:.0}% of frame height", thigh / height * 100.0)
            } else {
                "Small in frame: move the camera closer".into()
            },
        });
    }

    let vis: Vec<f64> = posed
        .iter()
        .flat_map(|f| CORE.iter().filter_map(move |&j| joint(f, side, j)))
        .map(|p| p.visibility)
        .collect();
    let clarity = vis.iter().sum::<f64>() / vis.len().max(1) as f64;
    checks.push(Check {
        key: "clarity",
        ok: clarity >= MIN_CLARITY,
        detail: if clarity >= MIN_CLARITY {
            format!("Landmarks clear ({:.0}%)", clarity * 100.0)
        } else {
            format!(
                "Landmarks uncertain ({:.0}%): light from the front, a plainer background, fitted clothes",
                clarity * 100.0
            )
        },
    });
    finish(frames.len(), fps, Some(side), checks)
}

fn finish(frames: usize, fps: f64, side: Option<Side>, checks: Vec<Check>) -> Framing {
    Framing {
        frames,
        fps,
        side,
        ok: checks.iter().all(|c| c.ok),
        checks,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const W: f64 = 1280.0;
    const H: f64 = 720.0;

    /// A pose from near-side joint positions in pixels (y down, as in the
    /// image). The far side sits `spread` px behind, a little less visible.
    pub(crate) fn pose_px(t_us: i64, near: Side, joints: [(f64, f64); 9], spread: f64) -> Pose {
        let side_points = |dx: f64, v: f32| -> Vec<[f32; 3]> {
            joints
                .iter()
                .map(|&(x, y)| [((x + dx) / W) as f32, (y / H) as f32, v])
                .collect()
        };
        let (near_pts, far_pts) = (side_points(0.0, 0.95), side_points(spread, 0.6));
        let points = match near {
            Side::Left => [near_pts, far_pts].concat(),
            Side::Right => [far_pts, near_pts].concat(),
        };
        Pose {
            captured_us: t_us,
            width: W as u16,
            height: H as u16,
            points,
        }
    }

    /// Rower facing right (feet to the right) at the catch: shins vertical,
    /// body leaning forward, arms straight.
    pub(crate) fn catch_joints() -> [(f64, f64); 9] {
        [
            (560.0, 250.0), // ear
            (560.0, 300.0), // shoulder: ahead of the hip
            (640.0, 380.0), // elbow
            (720.0, 460.0), // wrist
            (480.0, 500.0), // hip
            (700.0, 360.0), // knee: right above the ankle
            (700.0, 560.0), // ankle
            (680.0, 580.0), // heel
            (760.0, 580.0), // toe
        ]
    }

    fn frames(n: usize, fps: f64, f: impl Fn(usize) -> Pose) -> Vec<Pose> {
        (0..n)
            .map(|i| Pose {
                captured_us: (i as f64 * 1e6 / fps) as i64,
                ..f(i)
            })
            .collect()
    }

    fn close(a: Option<f64>, b: f64, tol: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < tol)
    }

    #[test]
    fn catch_angles() {
        let p = pose_px(0, Side::Left, catch_joints(), 10.0);
        let a = angles(&p, Side::Left, 0.0);
        assert!(close(a.shin, 0.0, 0.1), "vertical shin: {:?}", a.shin);
        // Hip→shoulder: 80 px forward, 200 px up → 21.8° toward the feet.
        assert!(close(a.trunk, 21.8, 0.1), "{:?}", a.trunk);
        assert!(close(a.elbow, 180.0, 0.1), "straight arm: {:?}", a.elbow);
        assert!(a.knee.unwrap() < 90.0, "compressed: {:?}", a.knee);
    }

    #[test]
    fn angles_dont_depend_on_which_way_the_rower_faces() {
        let mirrored = catch_joints().map(|(x, y)| (W - x, y));
        let a = angles(&pose_px(0, Side::Right, catch_joints(), 10.0), Side::Right, 0.0);
        let b = angles(&pose_px(0, Side::Left, mirrored, -10.0), Side::Left, 0.0);
        for (x, y) in [(a.knee, b.knee), (a.trunk, b.trunk), (a.shin, b.shin), (a.elbow, b.elbow), (a.hip, b.hip)] {
            assert!((x.unwrap() - y.unwrap()).abs() < 1e-9);
        }
    }

    #[test]
    fn layback_is_negative_and_unseen_joints_give_none() {
        let mut j = catch_joints();
        j[1] = (400.0, 310.0); // shoulder behind the hip
        let mut p = pose_px(0, Side::Left, j, 10.0);
        assert!(angles(&p, Side::Left, 0.0).trunk.unwrap() < 0.0);
        p.points[Wrist as usize][2] = 0.1;
        assert_eq!(angles(&p, Side::Left, 0.0).elbow, None);
    }

    #[test]
    fn tilt_comes_from_the_hip_path() {
        // Hip slides 200 px right while the image is rolled 5°.
        let tilt: f64 = 5.0;
        let fr: Vec<Pose> = (0..30)
            .map(|i| {
                let dx = i as f64 * 7.0;
                let mut j = catch_joints();
                for p in &mut j {
                    p.0 += dx;
                    p.1 -= dx * tilt.to_radians().tan();
                }
                pose_px(i, Side::Left, j, 10.0)
            })
            .collect();
        let est = camera_tilt(&fr, Side::Left).unwrap();
        assert!((est - tilt).abs() < 0.01, "tilt {est}");
        // Not enough travel: no estimate.
        assert_eq!(camera_tilt(&fr[..2], Side::Left), None);
    }

    #[test]
    fn levelling_undoes_a_rolled_camera() {
        // Roll the whole scene 7° counter-clockwise (y up) about the image origin.
        let tilt: f64 = 7.0;
        let (s, c) = tilt.to_radians().sin_cos();
        let rolled = catch_joints().map(|(x, y)| {
            let (px, py) = (x, -y);
            (px * c - py * s, -(px * s + py * c))
        });
        let level_angles = angles(&pose_px(0, Side::Left, catch_joints(), 10.0), Side::Left, 0.0);
        let p = pose_px(0, Side::Left, rolled, 10.0);
        let uncorrected = angles(&p, Side::Left, 0.0);
        let corrected = angles(&p, Side::Left, tilt);
        assert!((uncorrected.trunk.unwrap() - level_angles.trunk.unwrap()).abs() > 5.0);
        assert!((corrected.trunk.unwrap() - level_angles.trunk.unwrap()).abs() < 0.01);
        assert!((corrected.shin.unwrap() - level_angles.shin.unwrap()).abs() < 0.01);
        // Angles at a joint don't care about roll.
        assert!((uncorrected.knee.unwrap() - level_angles.knee.unwrap()).abs() < 0.01);
    }

    #[test]
    fn near_side_is_the_clearer_one() {
        let p = pose_px(0, Side::Right, catch_joints(), 10.0);
        assert_eq!(near_side([&p]), Some(Side::Right));
        assert_eq!(near_side(std::iter::empty()), None);
    }

    #[test]
    fn good_setup_passes() {
        let fr = frames(30, 30.0, |_| pose_px(0, Side::Left, catch_joints(), 10.0));
        let f = framing(&fr);
        assert!(f.ok, "{:#?}", f.checks);
        assert_eq!(f.side, Some(Side::Left));
        assert!((f.fps - 30.0).abs() < 0.5);
    }

    #[test]
    fn setup_problems_are_named() {
        let failing = |fr: &[Pose]| -> Vec<&'static str> {
            framing(fr).checks.iter().filter(|c| !c.ok).map(|c| c.key).collect()
        };
        // Feet cut off at the bottom of the frame.
        let mut low = catch_joints();
        for p in &mut low[6..] {
            p.1 = 715.0;
        }
        let fr = frames(30, 30.0, |_| pose_px(0, Side::Left, low, 10.0));
        assert_eq!(failing(&fr), vec!["body"]);
        assert!(framing(&fr).checks[2].detail.contains("ankle, heel, toe"));

        // Camera at an angle: far shoulder well apart.
        let fr = frames(30, 30.0, |_| pose_px(0, Side::Left, catch_joints(), 150.0));
        assert_eq!(failing(&fr), vec!["side_on"]);

        // Slow camera, nobody there.
        let fr = frames(10, 10.0, |_| Pose {
            width: 1280,
            height: 720,
            ..Default::default()
        });
        assert_eq!(failing(&fr), vec!["camera", "person"]);
        assert!(framing(&[]).checks[0].detail.contains("No frames"));
    }
}
