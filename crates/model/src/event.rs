use serde::{Deserialize, Serialize};

/// One timestamped observation from a device or from the app itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Microseconds since the Unix epoch, from [`crate::Clock`], stamped when
    /// the observation reached the app.
    pub t_us: i64,
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    /// One PM5 status poll.
    Pm5Status(Pm5Status),
    /// A chunk of force-curve samples drained from the PM5's buffer.
    Pm5Force(Pm5Force),
    /// One heart-rate notification from a BLE strap.
    Heart(Heart),
    /// A device connected or dropped.
    Link(Link),
    /// A user annotation, e.g. "early arms on purpose" for fault fixtures.
    Marker { label: String },
    /// Body landmarks from one camera frame, detected in the browser.
    Pose(Pose),
}

/// PM5 status as reported by one poll. Fields are `None` when the PM5 left
/// that command out of its reply.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pm5Status {
    pub stroke_state: Option<StrokeState>,
    /// Strokes per minute.
    pub spm: Option<u16>,
    /// Power of the last stroke, watts.
    pub watts: Option<u16>,
    /// Work distance, decimetres (0.1 m).
    pub distance_dm: Option<u32>,
    /// Work time, centiseconds (0.01 s).
    pub work_time_cs: Option<u32>,
    pub drag_factor: Option<u8>,
    /// Raw PM workout state code.
    pub workout_state: Option<u8>,
}

/// Force-curve samples, raw from the PM5 (pounds-force), in order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pm5Force {
    pub stroke_state: Option<StrokeState>,
    pub points: Vec<u16>,
}

/// PM5 stroke state (`CSAFE_PM_GET_STROKESTATE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokeState {
    /// Flywheel below minimum speed: not rowing.
    WaitingForMinSpeed,
    /// Flywheel spinning, waiting for it to accelerate (the catch).
    WaitingForAcceleration,
    Drive,
    /// Short pause after the drive.
    Dwell,
    Recovery,
    Unknown(u8),
}

impl StrokeState {
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::WaitingForMinSpeed,
            1 => Self::WaitingForAcceleration,
            2 => Self::Drive,
            3 => Self::Dwell,
            4 => Self::Recovery,
            other => Self::Unknown(other),
        }
    }

    /// True once the flywheel has dropped below rowing speed.
    pub fn is_idle(self) -> bool {
        self == Self::WaitingForMinSpeed
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::WaitingForMinSpeed => "idle",
            Self::WaitingForAcceleration => "catch",
            Self::Drive => "drive",
            Self::Dwell => "dwell",
            Self::Recovery => "recovery",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// One BLE Heart Rate Measurement (0x2A37) notification.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Heart {
    pub bpm: u16,
    /// Beat-to-beat intervals in 1/1024 s, as the strap sends them.
    pub rr_1024: Vec<u16>,
    /// Skin contact, when the strap reports it.
    pub contact: Option<bool>,
}

impl Heart {
    pub fn rr_ms(&self) -> impl Iterator<Item = f64> + '_ {
        self.rr_1024.iter().map(|&rr| rr as f64 * 1000.0 / 1024.0)
    }
}

/// Joints kept from the pose model, in the order [`Pose::points`] uses for
/// each side. Enough for knee, hip, elbow, trunk and shin angles.
pub const POSE_JOINTS: [&str; 9] = ["ear", "shoulder", "elbow", "wrist", "hip", "knee", "ankle", "heel", "toe"];

/// One camera frame's landmarks. Coordinates are fractions of the frame
/// (x right, y down), so `width`/`height` are needed to measure angles.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    /// When the frame was captured, µs since the Unix epoch, by the browser's
    /// clock (`performance.timeOrigin + captureTime`). The event's `t_us` is
    /// when it reached the app; this is the one to align with the PM5.
    pub captured_us: i64,
    pub width: u16,
    pub height: u16,
    /// `[x, y, visibility]` for each of [`POSE_JOINTS`] on the body's left
    /// side, then the same for the right side.
    pub points: Vec<[f32; 3]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    Pm5,
    H10,
    Camera,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub device: Device,
    pub connected: bool,
    /// Device name/path when connecting, error when dropping.
    pub detail: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stroke_state_codes_round_trip() {
        for code in 0..=6u8 {
            let state = StrokeState::from_code(code);
            match state {
                StrokeState::Unknown(c) => assert_eq!(c, code),
                _ => assert!(code <= 4),
            }
        }
        assert!(StrokeState::from_code(0).is_idle());
        assert!(!StrokeState::from_code(1).is_idle());
    }

    #[test]
    fn rr_converts_to_ms() {
        let h = Heart {
            bpm: 60,
            rr_1024: vec![1024, 512],
            contact: None,
        };
        assert_eq!(h.rr_ms().collect::<Vec<_>>(), vec![1000.0, 500.0]);
    }
}
