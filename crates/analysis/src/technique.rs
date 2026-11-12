//! Technique per stroke: the camera's view of each stroke lined up with the
//! PM5's, and the faults it shows (DESIGN.md §5.1, faults 1–4, 7 and 9).
//!
//! Thresholds start from common coaching norms and are to be tuned on
//! deliberately-rowed recordings.

use std::collections::VecDeque;

use model::{Event, EventKind, Pose};
use serde::{Deserialize, Serialize};

use crate::ideal::{Body, Direction, Guide, Phase, Sample, Sequence, sample, spans};
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
/// The body has started to swing back before the handle has come this share
/// of the drive: the good stroke's body waits until about a third.
pub const EARLY_BODY_START: f64 = 0.2;
/// The knees have started up before the handle has gone this share of the
/// way forward: the good stroke's wait until about two thirds.
pub const EARLY_KNEES_START: f64 = 0.5;
/// Body angle a good stroke keeps to (degrees, + toward the feet): forward
/// at the catch, back at the finish (DESIGN.md §5.1, faults 5 and 6). The
/// mirror's target leans as the rower does, held inside these.
pub const CATCH_TRUNK_GUIDE: (f64, f64) = (15.0, 30.0);
pub const FINISH_TRUNK_GUIDE: (f64, f64) = (-30.0, -5.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fault {
    /// Doesn't reach full compression: shins short of vertical at the catch.
    ShortCompression,
    /// Seat moves before the handle: the trunk tips forward early in the drive.
    ShootingSlide,
    /// The body swings back from the catch, instead of after the legs.
    EarlyBody,
    /// Arms bend while the legs are still driving.
    EarlyArms,
    /// Recovery under 1.5× the drive at a steady rate.
    RushedRecovery,
    /// Knees come up before the hands are away and the body is over.
    EarlyKnees,
}

impl Fault {
    /// In the order they weaken a stroke.
    pub const ALL: [Fault; 6] = [
        Fault::ShortCompression,
        Fault::ShootingSlide,
        Fault::EarlyBody,
        Fault::EarlyArms,
        Fault::RushedRecovery,
        Fault::EarlyKnees,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ShortCompression => "short compression",
            Self::ShootingSlide => "shooting the slide",
            Self::EarlyBody => "early body",
            Self::EarlyArms => "early arms",
            Self::RushedRecovery => "rushed recovery",
            Self::EarlyKnees => "early knees",
        }
    }

    /// What a coach would say (≤ 4 words).
    pub fn cue(self) -> &'static str {
        match self {
            Self::ShortCompression => "Get to full compression",
            Self::ShootingSlide => "Handle moves with the seat",
            Self::EarlyBody => "Legs, then swing",
            Self::EarlyArms => "Legs first, arms long",
            Self::RushedRecovery => "Slow the slide",
            Self::EarlyKnees => "Hands, body, then slide",
        }
    }

    /// The `--focus` value that selects it on the command line.
    pub fn flag(self) -> &'static str {
        match self {
            Self::ShortCompression => "short-compression",
            Self::ShootingSlide => "shooting-slide",
            Self::EarlyBody => "early-body",
            Self::EarlyArms => "early-arms",
            Self::RushedRecovery => "rushed-recovery",
            Self::EarlyKnees => "early-knees",
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
    /// When the legs, the body and the arms each moved, by the handle.
    pub sequence: Sequence,
}

/// How long the handle took, by the camera: `(drive, recovery)` in ms, from
/// the camera catch (wrist furthest forward) to its finish (furthest back),
/// and the rest of the stroke's cycle. The PM5's own drive time only covers
/// the flywheel speeding up, about half the handle's drive.
pub fn handle_timing(stroke: &StrokeRecord, pose: Option<&StrokePose>) -> Option<(f64, f64)> {
    let drive = pose.map(|p| p.finish_ms - p.catch_ms).filter(|&d| d > 0.0)?;
    let cycle = stroke.cycle_s()? * 1000.0;
    (cycle > drive).then_some((drive, cycle - drive))
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

/// A frame's near-side body relative to the hip, in thigh lengths, x toward the feet.
pub fn figure(f: &Pose, view: &CameraView) -> Figure {
    let Some(hip) = at(f, view, Joint::Hip) else {
        return Vec::new();
    };
    figure_from(f, view, hip)
}

/// Where the feet rest, levelled: the median ankle. The feet don't move on
/// the footplate, but the ankle is often the least certain point, at the
/// bottom of the picture.
fn feet(frames: &[Pose], view: &CameraView) -> Option<(f64, f64)> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for (x, y) in frames.iter().filter_map(|f| at(f, view, Joint::Ankle)) {
        xs.push(x);
        ys.push(y);
    }
    if xs.len() < 10 {
        return None;
    }
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    Some((xs[xs.len() / 2], ys[ys.len() / 2]))
}

/// The frames from the first stroke on (with a margin), or all of them
/// before there's a stroke: climbing on and off the erg moves the hip in
/// ways rowing doesn't, which would read as a tilted camera.
pub fn rowing_frames<'a>(strokes: &[StrokeRecord], frames: &'a [Pose]) -> &'a [Pose] {
    let (Some(first), Some(last)) = (strokes.first(), strokes.last()) else {
        return frames;
    };
    let end = last.t_next_catch_us.unwrap_or(last.t_catch_us) + ROWING_MARGIN_US;
    let lo = first_at(frames, first.t_catch_us - ROWING_MARGIN_US);
    let hi = first_at(frames, end).max(lo);
    &frames[lo..hi]
}

/// Frames this long either side of the strokes count as rowing.
const ROWING_MARGIN_US: i64 = 2_000_000;

/// A frame's near-side body relative to `origin` (levelled pixels), in thigh
/// lengths, x toward the feet, y up.
fn figure_from(f: &Pose, view: &CameraView, origin: (f64, f64)) -> Figure {
    use Joint::*;
    [Ear, Shoulder, Elbow, Wrist, Hip, Knee, Ankle, Heel, Toe]
        .iter()
        .map(|&j| {
            at(f, view, j).map(|(x, y)| {
                [
                    ((x - origin.0) * view.facing / view.thigh_px) as f32,
                    ((y - origin.1) / view.thigh_px) as f32,
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
    // The three parts frame by frame: the drive, then the recovery up to
    // where this stroke's frames end, just short of the next catch.
    let through = |range: std::ops::Range<usize>| -> Vec<Sample> {
        range
            .filter_map(|i| {
                let a = ang(i);
                Some(Sample {
                    handle: reach(&frames[i], view)? / view.thigh_px,
                    seat: at(&frames[i], view, Joint::Hip)?.0 * view.facing / view.thigh_px,
                    lean: a.trunk?,
                    elbow: a.elbow?,
                })
            })
            .collect()
    };
    let sequence = Sequence {
        drive: spans(&through(catch_i..finish_i + 1)),
        recovery: spans(&through(finish_i..hi)),
    };

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
        sequence,
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
        if let Some(body) = p.sequence.drive.body.filter(|b| b.start < EARLY_BODY_START) {
            hit(Fault::EarlyBody, body.start);
        }
        if let Some(elbow) = p.elbow_while_legs_bent.filter(|&e| e < EARLY_ARMS_ELBOW_DEG) {
            hit(Fault::EarlyArms, elbow);
        }
        if let Some(legs) = p.sequence.recovery.legs.filter(|l| l.start < EARLY_KNEES_START) {
            hit(Fault::EarlyKnees, legs.start);
        }
    }
    let ratio = handle_timing(stroke, pose).map(|(drive, recovery)| recovery / drive).or(stroke.ratio);
    if let (Some(ratio), Some(rate)) = (ratio, stroke.rate())
        && rate <= RUSHED_MAX_SPM
        && ratio < RUSHED_RATIO
    {
        hit(Fault::RushedRecovery, ratio);
    }
    hits
}

/// Technique as the session happens: buffers recent frames, calibrates the
/// sync offset as strokes arrive, and gives each completed stroke its pose
/// and faults. Converges on what [`apply`] finds after the fact.
#[derive(Debug, Default)]
pub struct LiveTechnique {
    frames: VecDeque<Pose>,
    candidates: Vec<f64>,
    strokes: usize,
    view: Option<CameraView>,
    /// Recent strokes' poses, for the typical figure.
    recent: VecDeque<StrokePose>,
    /// Frames from here on are rowing (the first stroke, less a margin).
    rowing_from_us: Option<i64>,
    /// Where the feet rest: the live figure is drawn from here, so it holds still.
    feet: Option<(f64, f64)>,
    /// Frames taken in so far.
    seen: usize,
    /// The good stroke on this rower's body, once enough of them is seen.
    body: Option<Body>,
    direction: Direction,
    /// The body's lean with the handle at its furthest so far this phase,
    /// and at the last catch and the last finish.
    turn_lean: Option<f64>,
    catch_lean: Option<f64>,
    finish_lean: Option<f64>,
}

/// Strokes the live typical figure is the median of.
const LIVE_TYPICAL: usize = 20;
/// Before the first stroke, the camera view (side, scale), where the feet
/// are and the rower's proportions are worked out again every this many
/// frames, from the last `SETTLING_US`: someone walking up and sitting down
/// is on the screen, to scale, a few seconds after they sit.
const FIRST_VIEW_FRAMES: usize = 30;
const SETTLING_US: i64 = 4_000_000;

/// Frames kept: several strokes, enough for side, tilt and scale.
const LIVE_BUFFER_US: i64 = 15_000_000;

impl LiveTechnique {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn frame(&mut self, pose: &Pose) {
        if !has_pose(pose) {
            return;
        }
        // Frames can arrive slightly out of capture order.
        let at = self.frames.partition_point(|f| f.captured_us <= pose.captured_us);
        self.frames.insert(at, pose.clone());
        let newest = self.frames.back().map_or(0, |f| f.captured_us);
        while self.frames.front().is_some_and(|f| newest - f.captured_us > LIVE_BUFFER_US) {
            self.frames.pop_front();
        }
        // Before the first stroke, a view from the frames alone, for the
        // mirror: side and scale, but no tilt, which only rowing shows. From
        // the first stroke on, each stroke works them out from the rowing.
        self.seen += 1;
        if self.strokes == 0 && self.seen.is_multiple_of(FIRST_VIEW_FRAMES) {
            let frames = self.frames.make_contiguous();
            let recent = &frames[first_at(frames, newest - SETTLING_US)..];
            if let Some(v) = camera_view(recent) {
                self.feet = feet(recent, &v);
                self.view = Some(CameraView { tilt_deg: 0.0, ..v });
                self.measure_body(newest - SETTLING_US);
            }
        }
    }

    /// The rower's proportions from the frames since `from_us`.
    fn measure_body(&mut self, from_us: i64) {
        let (Some(v), Some(origin)) = (&self.view, self.feet) else { return };
        let figures: Vec<Figure> = self
            .frames
            .iter()
            .filter(|f| f.captured_us >= from_us)
            .step_by(2)
            .map(|f| figure_from(f, v, origin))
            .collect();
        if let Some(mut body) = Body::measure(&figures) {
            body.set_lean(self.catch_lean, self.finish_lean);
            self.body = Some(body);
        }
    }

    /// A live figure against the good stroke: the target body with the
    /// handle where the rower's is, and where each part should be. The
    /// target leans at the catch and the finish as the rower last did
    /// (within the guides), so through the stroke it differs only in the
    /// order the parts move.
    pub fn guide(&mut self, figure: &Figure) -> Option<Guide> {
        let s = sample(figure)?;
        let moved = self.direction.push(s.handle);
        if moved.turned {
            match moved.phase {
                Phase::Drive => self.catch_lean = self.turn_lean,
                Phase::Recovery => self.finish_lean = self.turn_lean,
            }
            if let Some(body) = &mut self.body {
                body.set_lean(self.catch_lean, self.finish_lean);
            }
        }
        if moved.furthest || moved.turned {
            self.turn_lean = Some(s.lean);
        }
        self.body.as_ref()?.guide(moved.phase, figure)
    }

    /// The good stroke on this rower: its catch and finish, and when each
    /// part moves in it.
    pub fn target(&self) -> Option<Target> {
        let body = self.body.as_ref()?;
        let (catch, finish) = body.ends();
        Some(Target {
            catch,
            finish,
            sequence: body.sequence(),
        })
    }

    /// A frame's body as the mirror draws it, once the camera view is known:
    /// thigh lengths from where the feet rest, x toward the feet, y up.
    pub fn figure(&self, pose: &Pose) -> Option<Figure> {
        let v = self.view.as_ref()?;
        if !has_pose(pose) {
            return None;
        }
        let origin = self.feet.or_else(|| at(pose, v, Joint::Ankle))?;
        Some(figure_from(pose, v, origin))
    }

    /// Median catch and finish over the last strokes the camera saw.
    pub fn typical(&self) -> (Figure, Figure) {
        let strokes: Vec<StrokeRecord> = self
            .recent
            .iter()
            .map(|p| StrokeRecord {
                pose: Some(p.clone()),
                ..StrokeRecord::empty()
            })
            .collect();
        typical_figures(&strokes)
    }

    /// The offset so far: median of the strokes calibrated on.
    pub fn offset_ms(&self) -> Option<f64> {
        let mut c = self.candidates.clone();
        (!c.is_empty()).then(|| {
            c.sort_by(f64::total_cmp);
            c[c.len() / 2]
        })
    }

    /// Attach pose and faults to a stroke that just completed.
    pub fn stroke(&mut self, s: &mut StrokeRecord) {
        let from = *self.rowing_from_us.get_or_insert(s.t_catch_us - ROWING_MARGIN_US);
        let frames = self.frames.make_contiguous();
        let rowing = &frames[first_at(frames, from)..];
        let view = camera_view(rowing);
        if let Some(v) = &view {
            self.feet = feet(rowing, v).or(self.feet);
        }
        if let Some(v) = &view
            && self.strokes >= SYNC_SKIP
            && self.candidates.len() < SYNC_STROKES
        {
            let lo = first_at(frames, s.t_catch_us - SYNC_BEFORE_US);
            let hi = first_at(frames, s.t_catch_us + SYNC_AFTER_US);
            if let Some(t) = reach_peak(frames, lo, hi, v) {
                self.candidates.push((s.t_catch_us as f64 - t) / 1000.0);
            }
        }
        self.strokes += 1;
        let offset_ms = self.offset_ms();
        let frames = self.frames.make_contiguous();
        if let Some(v) = view {
            self.view = Some(CameraView { offset_ms, ..v });
        }
        s.pose = self.view.as_ref().and_then(|v| stroke_pose(s, frames, v));
        s.faults = faults(s, s.pose.as_ref());
        if let Some(p) = &s.pose {
            if self.recent.len() == LIVE_TYPICAL {
                self.recent.pop_front();
            }
            self.recent.push_back(p.clone());
        }
        self.measure_body(from);
    }
}

/// The good stroke on one rower's body.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Target {
    pub catch: Figure,
    pub finish: Figure,
    pub sequence: Sequence,
}

/// The good stroke's order for the rower of these strokes, from their
/// catches and finishes: what the review holds each stroke's sequence
/// against. `None` without enough strokes the camera saw.
pub fn target_sequence(strokes: &[StrokeRecord]) -> Option<Sequence> {
    let poses: Vec<&StrokePose> = strokes.iter().filter_map(|s| s.pose.as_ref()).collect();
    // Stored figures are from the hip; the body is measured from the feet.
    let from_feet = |f: &Figure| -> Option<Figure> {
        let [ax, ay] = f.get(Joint::Ankle as usize).copied().flatten()?;
        Some(f.iter().map(|p| p.map(|[x, y]| [x - ax, y - ay])).collect())
    };
    let figures: Vec<Figure> = poses
        .iter()
        .flat_map(|p| [&p.catch_figure, &p.finish_figure])
        .filter_map(from_feet)
        .collect();
    let median = |pick: fn(&StrokePose) -> Option<f64>| {
        let mut xs: Vec<f64> = poses.iter().filter_map(|p| pick(p)).collect();
        xs.sort_by(f64::total_cmp);
        xs.get(xs.len() / 2).copied()
    };
    let mut body = Body::measure(&figures)?;
    body.set_lean(median(|p| p.catch.trunk), median(|p| p.finish.trunk));
    Some(body.sequence())
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
    let view = camera_view(rowing_frames(strokes, &frames)).map(|mut v| {
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
        // Finish: hands drawn in to the ribs, legs flat.
        assert!(p.finish.elbow.unwrap() < 100.0, "finish elbow {:?}", p.finish.elbow);
        assert!(p.catch.elbow.unwrap() > 170.0, "catch elbow {:?}", p.catch.elbow);
        assert!(p.finish.knee.unwrap() > 165.0, "finish knee {:?}", p.finish.knee);
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
            (SimFault::EarlyBody, Fault::EarlyBody),
            (SimFault::EarlyKnees, Fault::EarlyKnees),
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
    fn live_finds_what_the_whole_session_finds() {
        for fault in [None, Some(SimFault::EarlyArms)] {
            let cfg = SimConfig {
                pose: Some(PoseSim {
                    fault,
                    ..Default::default()
                }),
                ..Default::default()
            };
            let events = generate(cfg, T0, Duration::from_secs(120));
            let after = crate::analyze(&events, None).strokes;

            let mut asm = crate::strokes::StrokeAssembler::new();
            let mut live = LiveTechnique::new();
            let mut strokes = Vec::new();
            for ev in &events {
                if let EventKind::Pose(p) = &ev.kind {
                    live.frame(p);
                }
                if let Some(mut s) = asm.push(ev) {
                    live.stroke(&mut s);
                    strokes.push(s);
                }
            }
            assert!((live.offset_ms().unwrap() - 60.0).abs() <= 17.0);
            // Once calibrated, live and after-the-fact agree stroke by stroke.
            for (l, a) in strokes.iter().zip(&after).skip(SYNC_SKIP + SYNC_MIN) {
                let kinds = |s: &StrokeRecord| s.faults.iter().map(|h| h.fault).collect::<Vec<_>>();
                assert_eq!(kinds(l), kinds(a), "stroke {} ({fault:?})", l.index);
            }
        }
    }

    #[test]
    fn climbing_onto_the_erg_doesnt_read_as_a_tilted_camera() {
        let cfg = SimConfig {
            pose: Some(PoseSim::default()),
            ..Default::default()
        };
        let mut events = generate(cfg, T0, Duration::from_secs(30));
        // Ten seconds of the body coming down onto the seat from up and to
        // the side before the first stroke: a third of the frames, as in a
        // real recording's pre-roll.
        let first = pose_frames(&events).into_iter().next().unwrap();
        let climb: Vec<Event> = (0..300)
            .map(|i| {
                let mut p = first.clone();
                let k = (300 - i) as f32 / 300.0;
                for pt in &mut p.points {
                    pt[0] -= 0.3 * k;
                    pt[1] -= 0.3 * k;
                }
                p.captured_us = T0 - 14_000_000 + i * 33_333;
                Event {
                    t_us: p.captured_us,
                    kind: EventKind::Pose(p),
                }
            })
            .collect();
        events.splice(0..0, climb);
        let all = camera_view(&pose_frames(&events)).unwrap();
        assert!(all.tilt_deg.abs() > 3.0, "the climb alone skews the fit: {}", all.tilt_deg);
        let a = crate::analyze(&events, None);
        let view = a.summary.unwrap().camera.unwrap();
        assert!(view.tilt_deg.abs() < 0.5, "only rowing counts: {}", view.tilt_deg);
    }

    /// What the mirror would have shown through a simulated row: every
    /// frame's guide, from the first stroke's end on.
    fn guides(fault: Option<SimFault>) -> (Vec<Guide>, LiveTechnique) {
        let cfg = SimConfig {
            pose: Some(PoseSim {
                fault,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut live = LiveTechnique::new();
        let mut asm = crate::strokes::StrokeAssembler::new();
        let mut strokes = 0;
        let mut out = Vec::new();
        for ev in &generate(cfg, T0, Duration::from_secs(40)) {
            if let EventKind::Pose(p) = &ev.kind {
                live.frame(p);
                let guide = live.figure(p).and_then(|f| live.guide(&f));
                if strokes >= 2 {
                    out.push(guide.expect("a guide for every frame once rowing"));
                }
            }
            if let Some(mut s) = asm.push(ev) {
                live.stroke(&mut s);
                strokes += 1;
            }
        }
        (out, live)
    }

    #[test]
    fn the_live_target_follows_a_clean_stroke_and_shows_what_is_off() {
        use crate::ideal::{Off, Phase, Step};
        let (clean, live) = guides(None);
        assert!(clean.len() > 600);
        let off = clean.iter().filter(|g| g.off != Off::default()).count();
        assert_eq!(off, 0, "nothing off on a clean stroke");
        // Every step of the stroke comes up, in order.
        let mut steps: Vec<Step> = clean.iter().map(|g| g.step).collect();
        steps.dedup();
        use Step::*;
        let cycle = [Legs, Body, Arms, ArmsAway, BodyOver, Slide];
        let start = cycle.iter().position(|s| *s == steps[0]).unwrap();
        for (i, s) in steps.iter().enumerate() {
            assert_eq!(*s, cycle[(start + i) % 6], "step {i} of {steps:?}");
        }
        assert!(steps.len() >= 6 * 8, "{} steps in 10 strokes", steps.len());
        // The target is the simulator's rower: catch with the shins vertical.
        let t = live.target().unwrap();
        let (knee, ankle) = (t.catch[Joint::Knee as usize].unwrap(), t.catch[Joint::Ankle as usize].unwrap());
        assert!((knee[0] - ankle[0]).abs() < 0.05, "knee {knee:?} over ankle {ankle:?}");
        assert!(t.sequence.drive.arms.unwrap().start > t.sequence.drive.body.unwrap().start);

        // Arms bent from the catch: off through the leg drive, on every stroke.
        let (early, _) = guides(Some(SimFault::EarlyArms));
        let arms_off = |g: &&Guide| g.phase == Phase::Drive && g.off.arms && g.you.arms > g.ideal.arms;
        assert!(early.iter().filter(arms_off).count() > 60, "{}", early.iter().filter(arms_off).count());
        // Knees up with the hands: the legs are ahead on the way forward. With
        // the handle matched that is a few centimetres of seat, so it shows
        // for a moment each stroke (the stroke's own sequence shows it whole).
        let (knees, _) = guides(Some(SimFault::EarlyKnees));
        let legs_off = |g: &&Guide| g.phase == Phase::Recovery && g.off.legs && g.you.legs < g.ideal.legs;
        assert!(knees.iter().filter(legs_off).count() >= 10, "{}", knees.iter().filter(legs_off).count());
    }

    #[test]
    fn the_target_is_there_before_the_first_stroke() {
        // Sitting at the catch, not rowing: a few seconds of the same frame.
        let joints = devices::sim_pose::figure(devices::sim_pose::Phase::Drive(0.0), None);
        let mut live = LiveTechnique::new();
        let mut guide = None;
        for i in 0..150 {
            let p = devices::sim_pose::pose_frame(T0 + i * 33_333, joints, || 0.0);
            live.frame(&p);
            guide = live.figure(&p).and_then(|f| live.guide(&f));
        }
        let g = guide.expect("a target to sit against");
        assert!(g.ideal.legs < 0.05 && g.ideal.body < 0.05 && g.ideal.arms < 0.05, "at the catch: {:?}", g.ideal);
        assert_eq!(g.off, crate::ideal::Off::default());
    }

    #[test]
    fn the_review_gets_the_good_stroke_s_order_from_stored_strokes() {
        let (strokes, _) = row(None, 0.33);
        let target = target_sequence(&strokes).unwrap();
        let mine = strokes[10].pose.as_ref().unwrap().sequence;
        for (name, mine, target) in [
            ("body", mine.drive.body, target.drive.body),
            ("arms", mine.drive.arms, target.drive.arms),
            ("knees", mine.recovery.legs, target.recovery.legs),
        ] {
            let (mine, target) = (mine.unwrap(), target.unwrap());
            assert!((mine.start - target.start).abs() < 0.12, "{name}: starts at {:.2}, target {:.2}", mine.start, target.start);
        }
        // Too few strokes seen to measure a body from.
        assert_eq!(target_sequence(&strokes[..5]), None);
        assert_eq!(target_sequence(&[]), None);
    }

    #[test]
    fn the_live_figure_stands_where_the_feet_rest() {
        let cfg = SimConfig {
            pose: Some(PoseSim::default()),
            ..Default::default()
        };
        let events = generate(cfg, T0, Duration::from_secs(30));
        let mut live = LiveTechnique::new();
        let mut asm = crate::strokes::StrokeAssembler::new();
        let mut hips = Vec::new();
        for ev in &events {
            if let EventKind::Pose(p) = &ev.kind {
                live.frame(p);
                if let Some(f) = live.figure(p) {
                    let ankle = f[6].unwrap();
                    assert!(ankle[0].abs() < 0.05 && ankle[1].abs() < 0.05, "feet at the origin: {ankle:?}");
                    hips.push(f[4].unwrap()[0]);
                }
            }
            if let Some(mut s) = asm.push(ev) {
                live.stroke(&mut s);
            }
        }
        // The seat slides behind the feet: from about one thigh to two.
        let (lo, hi) = hips.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
        assert!(hi < -0.5 && lo > -2.5 && hi - lo > 0.6, "hip from {lo} to {hi}");
    }

    #[test]
    fn the_camera_times_the_handle_not_the_flywheel() {
        let (strokes, _) = row(None, 0.33);
        let s = &strokes[10];
        let (drive, recovery) = handle_timing(s, s.pose.as_ref()).unwrap();
        let cycle = s.cycle_s().unwrap() * 1000.0;
        assert!((drive / cycle - 0.33).abs() < 0.05, "drive {drive} of {cycle} ms");
        assert!((drive + recovery - cycle).abs() < 1.0);
        assert_eq!(handle_timing(s, None), None);
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
