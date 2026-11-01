//! Cutting the PM5 event stream into strokes.
//!
//! The PM5's stroke state marks the drive. A drive's force samples keep
//! arriving after it ends (the buffer drains on the next polls), so every
//! sample from one drive start up to the next belongs to that drive.
//!
//! [`CurveAssembler`] cuts raw drive curves; [`StrokeAssembler`] builds on it
//! and attaches the PM5's per-stroke numbers and heart rate, giving one
//! [`StrokeRecord`] per stroke.

use model::{Event, EventKind, StrokeState};
use serde::{Deserialize, Serialize};

use crate::metrics::LBF_TO_N;

/// One drive's force curve plus the timing around it.
#[derive(Debug, Clone, PartialEq)]
pub struct DriveCurve {
    pub index: u32,
    /// First event that reported the drive.
    pub drive_start_us: i64,
    /// First event after the drive that reported another state.
    pub drive_end_us: Option<i64>,
    /// Start of the next drive; `None` if rowing stopped after this stroke.
    pub next_drive_us: Option<i64>,
    /// Raw PM5 force samples (lbf), in order.
    pub points: Vec<u16>,
}

impl DriveCurve {
    pub fn drive_ms(&self) -> Option<f64> {
        Some((self.drive_end_us? - self.drive_start_us) as f64 / 1000.0)
    }

    pub fn recovery_ms(&self) -> Option<f64> {
        Some((self.next_drive_us? - self.drive_end_us?) as f64 / 1000.0)
    }

    /// Recovery time over drive time (target is about 2 at steady rates).
    pub fn ratio(&self) -> Option<f64> {
        let drive = self.drive_ms()?;
        (drive > 0.0).then(|| self.recovery_ms().map(|r| r / drive)).flatten()
    }

    pub fn peak_lbf(&self) -> Option<u16> {
        self.points.iter().copied().max()
    }
}

/// Incremental: push events as they arrive, get each stroke once it's complete.
#[derive(Debug, Default)]
pub struct CurveAssembler {
    current: Option<DriveCurve>,
    last_state: Option<StrokeState>,
    next_index: u32,
    orphan_points: usize,
}

impl CurveAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the previous stroke when a new drive starts, or when the
    /// flywheel stops.
    pub fn push(&mut self, ev: &Event) -> Option<DriveCurve> {
        let (state, points) = match &ev.kind {
            EventKind::Pm5Status(s) => (s.stroke_state, None),
            EventKind::Pm5Force(f) => (f.stroke_state, Some(f.points.as_slice())),
            _ => return None,
        };

        let mut done = None;
        // Unknown codes carry no timing information; skip them entirely.
        if let Some(state) = state.filter(|s| !matches!(s, StrokeState::Unknown(_))) {
            let was_drive = self.last_state == Some(StrokeState::Drive);
            if state == StrokeState::Drive && !was_drive {
                done = self.close(Some(ev.t_us), Some(ev.t_us));
                self.current = Some(DriveCurve {
                    index: self.next_index,
                    drive_start_us: ev.t_us,
                    drive_end_us: None,
                    next_drive_us: None,
                    points: Vec::new(),
                });
                self.next_index += 1;
            } else if state != StrokeState::Drive
                && was_drive
                && let Some(c) = &mut self.current
            {
                c.drive_end_us.get_or_insert(ev.t_us);
            }
            if state.is_idle() {
                done = done.or_else(|| self.close(None, Some(ev.t_us)));
            }
            self.last_state = Some(state);
        }

        if let Some(points) = points {
            match &mut self.current {
                Some(c) => c.points.extend_from_slice(points),
                None => self.orphan_points += points.len(),
            }
        }
        done
    }

    /// The stroke in progress, if any.
    pub fn current(&self) -> Option<&DriveCurve> {
        self.current.as_ref()
    }

    /// The stroke still in progress, if any (call at the end of a stream).
    pub fn finish(&mut self) -> Option<DriveCurve> {
        self.close(None, None)
    }

    /// Force samples that arrived with no drive to attach them to.
    pub fn orphan_points(&self) -> usize {
        self.orphan_points
    }

    /// `at`: when the drive must have ended at the latest, if known.
    fn close(&mut self, next_drive_us: Option<i64>, at: Option<i64>) -> Option<DriveCurve> {
        let mut c = self.current.take()?;
        if c.drive_end_us.is_none() {
            c.drive_end_us = at;
        }
        c.next_drive_us = next_drive_us;
        Some(c)
    }
}

/// All strokes in a recorded stream.
pub fn drive_curves<'a>(events: impl IntoIterator<Item = &'a Event>) -> (Vec<DriveCurve>, usize) {
    let mut asm = CurveAssembler::new();
    let mut out: Vec<_> = events.into_iter().filter_map(|ev| asm.push(ev)).collect();
    out.extend(asm.finish());
    (out, asm.orphan_points())
}

/// Force curves are resampled to this many points so strokes line up point by point.
pub const CURVE_POINTS: usize = 50;

/// A heart reading this old still stands in for a stroke that had none.
const HR_HOLD_US: i64 = 5_000_000;

/// Everything known about one stroke, catch to next catch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrokeRecord {
    pub index: u32,
    pub t_catch_us: i64,
    pub t_finish_us: Option<i64>,
    /// `None` when rowing stopped after this stroke.
    pub t_next_catch_us: Option<i64>,
    pub drive_ms: Option<f64>,
    pub recovery_ms: Option<f64>,
    pub ratio: Option<f64>,
    /// PM5 power for the stroke.
    pub watts: Option<u16>,
    /// PM5 stroke rate (whole numbers, smoothed); [`StrokeRecord::rate`] is this stroke's own.
    pub spm: Option<u16>,
    /// Distance the PM5 credited from this catch to the next.
    pub distance_m: Option<f64>,
    pub drag_factor: Option<u8>,
    /// Mean heart rate over the stroke.
    pub hr_bpm: Option<f64>,
    pub peak_n: Option<f64>,
    /// Drive force resampled to [`CURVE_POINTS`] points, newtons.
    pub force_n: Vec<f64>,
}

impl StrokeRecord {
    /// Catch to next catch, seconds.
    pub fn cycle_s(&self) -> Option<f64> {
        Some((self.t_next_catch_us? - self.t_catch_us) as f64 / 1e6)
    }

    /// Strokes per minute from this stroke's own timing.
    pub fn rate(&self) -> Option<f64> {
        self.cycle_s().filter(|&c| c > 0.0).map(|c| 60.0 / c)
    }

    /// Work done in this stroke: its power over its own duration, or over
    /// `60 / spm` when the next catch is unknown.
    pub fn joules(&self) -> Option<f64> {
        let watts = self.watts? as f64;
        let secs = self
            .cycle_s()
            .or_else(|| self.spm.filter(|&s| s > 0).map(|s| 60.0 / s as f64))?;
        Some(watts * secs)
    }
}

/// Incremental: push every event, get each stroke's record once it's complete.
#[derive(Debug, Default)]
pub struct StrokeAssembler {
    curves: CurveAssembler,
    // Latest PM5 values.
    watts: Option<u16>,
    spm: Option<u16>,
    drag_factor: Option<u8>,
    distance_dm: Option<u32>,
    // The stroke in progress.
    catch_distance_dm: Option<u32>,
    hr_sum: f64,
    hr_n: u32,
    last_hr: Option<(i64, u16)>,
}

impl StrokeAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, ev: &Event) -> Option<StrokeRecord> {
        let before = self.curves.current().map(|c| c.index);
        // Close the finished stroke with the values seen during it, before
        // this event's values (which belong to the next stroke) are applied.
        let closing_distance = match &ev.kind {
            EventKind::Pm5Status(s) => s.distance_dm,
            _ => None,
        };
        let done = self.curves.push(ev).map(|c| self.record(c, closing_distance));

        match &ev.kind {
            EventKind::Pm5Status(s) => {
                // Assumes the PM5 reports a stroke's power and rate by the end
                // of its recovery. Check against real recordings.
                self.watts = s.watts.or(self.watts);
                self.spm = s.spm.or(self.spm);
                self.drag_factor = s.drag_factor.or(self.drag_factor);
                self.distance_dm = s.distance_dm.or(self.distance_dm);
            }
            EventKind::Heart(h) if h.bpm > 0 && h.contact != Some(false) => {
                self.last_hr = Some((ev.t_us, h.bpm));
                if self.curves.current().is_some() {
                    self.hr_sum += h.bpm as f64;
                    self.hr_n += 1;
                }
            }
            _ => {}
        }

        let after = self.curves.current().map(|c| c.index);
        if after.is_some() && after != before {
            self.catch_distance_dm = self.distance_dm;
        }
        done
    }

    /// The stroke still in progress, if any (call at the end of a stream).
    pub fn finish(&mut self) -> Option<StrokeRecord> {
        self.curves.finish().map(|c| self.record(c, None))
    }

    pub fn orphan_points(&self) -> usize {
        self.curves.orphan_points()
    }

    fn record(&mut self, c: DriveCurve, closing_distance: Option<u32>) -> StrokeRecord {
        let end = closing_distance.or(self.distance_dm);
        let distance_m = self
            .catch_distance_dm
            .zip(end)
            .filter(|(a, b)| b >= a)
            .map(|(a, b)| (b - a) as f64 / 10.0);
        let hr_bpm = if self.hr_n > 0 {
            Some(self.hr_sum / self.hr_n as f64)
        } else {
            self.last_hr
                .filter(|(t, _)| c.drive_start_us - t <= HR_HOLD_US)
                .map(|(_, bpm)| bpm as f64)
        };
        self.hr_sum = 0.0;
        self.hr_n = 0;
        self.catch_distance_dm = None;
        StrokeRecord {
            index: c.index,
            t_catch_us: c.drive_start_us,
            t_finish_us: c.drive_end_us,
            t_next_catch_us: c.next_drive_us,
            drive_ms: c.drive_ms(),
            recovery_ms: c.recovery_ms(),
            ratio: c.ratio(),
            watts: self.watts,
            spm: self.spm,
            distance_m,
            drag_factor: self.drag_factor,
            hr_bpm,
            peak_n: c.peak_lbf().map(|p| p as f64 * LBF_TO_N),
            force_n: resample(&c.points, CURVE_POINTS)
                .into_iter()
                .map(|v| v * LBF_TO_N)
                .collect(),
        }
    }
}

/// All stroke records in a recorded stream.
pub fn stroke_records<'a>(events: impl IntoIterator<Item = &'a Event>) -> Vec<StrokeRecord> {
    let mut asm = StrokeAssembler::new();
    let mut out: Vec<_> = events.into_iter().filter_map(|ev| asm.push(ev)).collect();
    out.extend(asm.finish());
    out
}

/// Linear resampling to `n` evenly spaced points (`n` ≥ 2).
pub fn resample(points: &[u16], n: usize) -> Vec<f64> {
    match points {
        [] => Vec::new(),
        [only] => vec![*only as f64; n],
        _ => {
            let last = (points.len() - 1) as f64;
            (0..n)
                .map(|j| {
                    let x = j as f64 * last / (n - 1) as f64;
                    let i = (x as usize).min(points.len() - 2);
                    let f = x - i as f64;
                    points[i] as f64 * (1.0 - f) + points[i + 1] as f64 * f
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Pm5Force, Pm5Status};

    fn status(t_ms: i64, state: StrokeState) -> Event {
        Event {
            t_us: t_ms * 1000,
            kind: EventKind::Pm5Status(Pm5Status {
                stroke_state: Some(state),
                ..Default::default()
            }),
        }
    }

    fn force(t_ms: i64, state: StrokeState, points: &[u16]) -> Event {
        Event {
            t_us: t_ms * 1000,
            kind: EventKind::Pm5Force(Pm5Force {
                stroke_state: Some(state),
                points: points.to_vec(),
            }),
        }
    }

    use StrokeState::*;

    #[test]
    fn late_samples_belong_to_the_drive_before() {
        let events = [
            status(0, Drive),
            force(50, Drive, &[10, 40]),
            status(1000, Recovery),
            force(1050, Recovery, &[15]), // drained after the drive ended
            status(3000, Drive),
            force(3050, Drive, &[12]),
        ];
        let (curves, orphans) = drive_curves(&events);
        assert_eq!(orphans, 0);
        assert_eq!(curves.len(), 2);
        let first = &curves[0];
        assert_eq!(first.points, vec![10, 40, 15]);
        assert_eq!(first.drive_ms(), Some(1000.0));
        assert_eq!(first.recovery_ms(), Some(2000.0));
        assert_eq!(first.ratio(), Some(2.0));
        assert_eq!(first.peak_lbf(), Some(40));
        // The last stroke is still open at the end of the stream.
        assert_eq!(curves[1].points, vec![12]);
        assert_eq!(curves[1].next_drive_us, None);
        assert_eq!(curves[1].drive_ms(), None);
    }

    #[test]
    fn emits_stroke_when_next_drive_starts() {
        let mut asm = CurveAssembler::new();
        assert!(asm.push(&status(0, Drive)).is_none());
        assert!(asm.push(&status(900, Dwell)).is_none());
        assert!(asm.push(&status(950, Recovery)).is_none());
        assert!(asm.push(&status(2900, WaitingForAcceleration)).is_none());
        let done = asm.push(&status(3000, Drive)).unwrap();
        assert_eq!(done.drive_ms(), Some(900.0));
        assert_eq!(done.recovery_ms(), Some(2100.0));
    }

    #[test]
    fn stopping_closes_the_last_stroke() {
        let mut asm = CurveAssembler::new();
        asm.push(&status(0, Drive));
        asm.push(&status(1000, Recovery));
        let done = asm.push(&status(5000, WaitingForMinSpeed)).unwrap();
        assert_eq!(done.next_drive_us, None);
        assert_eq!(done.recovery_ms(), None);
        assert!(asm.finish().is_none());
    }

    #[test]
    fn drive_straight_into_idle_still_gets_an_end() {
        let mut asm = CurveAssembler::new();
        asm.push(&status(0, Drive));
        let done = asm.push(&status(700, WaitingForMinSpeed)).unwrap();
        assert_eq!(done.drive_ms(), Some(700.0));
    }

    #[test]
    fn samples_before_any_drive_are_orphans() {
        let (curves, orphans) = drive_curves(&[force(0, Recovery, &[5, 6]), status(10, Recovery)]);
        assert!(curves.is_empty());
        assert_eq!(orphans, 2);
    }

    #[test]
    fn unknown_states_dont_split_a_drive() {
        let events = [
            status(0, Drive),
            status(50, Unknown(9)),
            status(100, Drive),
            status(900, Recovery),
            status(3000, Drive),
        ];
        let (curves, _) = drive_curves(&events);
        assert_eq!(curves.len(), 2);
        assert_eq!(curves[0].drive_ms(), Some(900.0));
    }

    fn pm5(t_ms: i64, state: StrokeState, watts: u16, distance_dm: u32) -> Event {
        Event {
            t_us: t_ms * 1000,
            kind: EventKind::Pm5Status(Pm5Status {
                stroke_state: Some(state),
                spm: Some(20),
                watts: Some(watts),
                distance_dm: Some(distance_dm),
                drag_factor: Some(110),
                ..Default::default()
            }),
        }
    }

    fn heart(t_ms: i64, bpm: u16) -> Event {
        Event {
            t_us: t_ms * 1000,
            kind: EventKind::Heart(model::Heart {
                bpm,
                rr_1024: vec![],
                contact: Some(true),
            }),
        }
    }

    #[test]
    fn records_carry_the_strokes_own_numbers() {
        let events = [
            pm5(0, Drive, 70, 100),
            force(50, Drive, &[10, 50, 20]),
            heart(500, 120),
            pm5(1000, Recovery, 84, 120), // power for this stroke, reported after the drive
            heart(1500, 124),
            pm5(3000, Drive, 84, 190),
            heart(3500, 200), // belongs to the second stroke
            pm5(4000, Recovery, 90, 210),
            pm5(6000, WaitingForMinSpeed, 0, 230),
        ];
        let recs = stroke_records(&events);
        assert_eq!(recs.len(), 2);
        let r = &recs[0];
        assert_eq!(r.watts, Some(84));
        assert_eq!(r.distance_m, Some(9.0));
        assert_eq!(r.hr_bpm, Some(122.0));
        assert_eq!(r.drag_factor, Some(110));
        assert_eq!(r.cycle_s(), Some(3.0));
        assert_eq!(r.rate(), Some(20.0));
        assert_eq!(r.joules(), Some(252.0));
        assert_eq!(r.force_n.len(), CURVE_POINTS);
        assert!((r.peak_n.unwrap() - 50.0 * LBF_TO_N).abs() < 1e-9);

        // Closed by the flywheel stopping: its power is the last one seen,
        // not the 0 the idle poll reports.
        let last = &recs[1];
        assert_eq!(last.watts, Some(90));
        assert_eq!(last.hr_bpm, Some(200.0));
        assert_eq!(last.distance_m, Some(4.0));
        assert_eq!(last.t_next_catch_us, None);
        assert_eq!(last.joules(), Some(270.0), "falls back to 60/spm");
    }

    #[test]
    fn stale_heart_rate_is_not_carried_forward() {
        let events = [heart(0, 130), pm5(1000, Drive, 80, 0), pm5(2000, Recovery, 80, 50), pm5(4000, Drive, 80, 100)];
        assert_eq!(stroke_records(&events)[0].hr_bpm, Some(130.0), "1 s old: still valid");
        let events = [heart(0, 130), pm5(9000, Drive, 80, 0), pm5(10000, Recovery, 80, 50), pm5(12000, Drive, 80, 100)];
        assert_eq!(stroke_records(&events)[0].hr_bpm, None, "9 s old: dropped");
    }

    #[test]
    fn lost_contact_readings_are_ignored() {
        let mut lost = heart(1500, 40);
        if let EventKind::Heart(h) = &mut lost.kind {
            h.contact = Some(false);
        }
        let events = [pm5(0, Drive, 80, 0), lost, pm5(1000, Recovery, 80, 50), pm5(3000, Drive, 80, 100)];
        assert_eq!(stroke_records(&events)[0].hr_bpm, None);
    }

    #[test]
    fn resample_keeps_ends_and_interpolates() {
        let r = resample(&[0, 10, 20], 5);
        assert_eq!(r, vec![0.0, 5.0, 10.0, 15.0, 20.0]);
        assert_eq!(resample(&[7], 3), vec![7.0; 3]);
        assert!(resample(&[], 3).is_empty());
        let long: Vec<u16> = (0..200).collect();
        let r = resample(&long, CURVE_POINTS);
        assert_eq!((r[0], r[CURVE_POINTS - 1]), (0.0, 199.0));
    }
}
