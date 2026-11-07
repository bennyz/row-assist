//! Whole-session numbers from stroke records and heart readings.

use serde::{Deserialize, Serialize};

use crate::metrics::{efficiency_factor, watts_from_pace};
use crate::strokes::{CURVE_POINTS, StrokeRecord};
use crate::technique::{CameraView, FaultRate, fault_rates, handle_timing};
use crate::zones::{HeartProfile, HeartStats, heart_stats};

/// Bump when the numbers below change meaning, so stored summaries can be
/// recomputed from their recordings.
/// 2: camera tilt from rowing frames only; drive time and ratio from the
/// camera's handle timing when there is a camera.
pub const ANALYSIS_VERSION: u32 = 2;

/// A catch-to-catch time longer than this (under 10 spm) is a pause, not a stroke.
pub const MAX_CYCLE_S: f64 = 6.0;
/// Decoupling skips the first minutes, where heart rate is still catching up.
pub const DECOUPLING_WARMUP_S: f64 = 180.0;
/// Decoupling needs this much steady rowing after the warm-up.
pub const DECOUPLING_MIN_S: f64 = 600.0;
/// Strokes count toward "watts at 20" inside this rate band.
pub const RATE_20: (f64, f64) = (19.5, 20.5);
/// "Watts at 20" needs this many seconds of strokes in the band.
pub const RATE_20_MIN_S: f64 = 120.0;
/// The last-10-minutes segment needs at least this long a session.
pub const LAST_10_MIN_SESSION_S: f64 = 900.0;

/// Averages over a set of strokes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Segment {
    pub strokes: u32,
    /// Catch to next catch, summed.
    pub seconds: f64,
    pub distance_m: Option<f64>,
    /// From the average pace when every stroke has a distance (as the PM5 and
    /// logbook compute it), else the time-weighted mean of stroke power.
    pub watts: Option<f64>,
    pub spm: Option<f64>,
    /// Time-weighted mean over strokes that have heart rate.
    pub hr: Option<f64>,
    /// Watts per bpm.
    pub ef: Option<f64>,
}

/// Strokes that make up rowing time: complete, and not spanning a pause.
fn rowing(s: &StrokeRecord) -> Option<f64> {
    s.cycle_s().filter(|&c| c > 0.0 && c <= MAX_CYCLE_S)
}

pub fn segment<'a>(strokes: impl IntoIterator<Item = &'a StrokeRecord>) -> Segment {
    let mut seg = Segment::default();
    let (mut dist, mut all_dist) = (0.0, true);
    let (mut w_sum, mut w_secs) = (0.0, 0.0);
    let (mut hr_sum, mut hr_secs) = (0.0, 0.0);
    for s in strokes {
        let Some(secs) = rowing(s) else { continue };
        seg.strokes += 1;
        seg.seconds += secs;
        match s.distance_m.filter(|&d| d > 0.0) {
            Some(d) => dist += d,
            None => all_dist = false,
        }
        if let Some(w) = s.watts {
            w_sum += w as f64 * secs;
            w_secs += secs;
        }
        if let Some(hr) = s.hr_bpm {
            hr_sum += hr * secs;
            hr_secs += secs;
        }
    }
    if seg.strokes == 0 {
        return seg;
    }
    if all_dist {
        seg.distance_m = Some(dist);
        seg.watts = Some(watts_from_pace(seg.seconds / dist * 500.0));
    } else if w_secs > 0.0 {
        seg.watts = Some(w_sum / w_secs);
    }
    seg.spm = Some(seg.strokes as f64 * 60.0 / seg.seconds);
    seg.hr = (hr_secs > 0.0).then(|| hr_sum / hr_secs);
    seg.ef = seg.watts.zip(seg.hr).and_then(|(w, hr)| efficiency_factor(w, hr));
    seg
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Summary {
    pub analysis_version: u32,
    /// First catch.
    pub started_at_us: i64,
    /// Last stroke's end (next catch, finish or catch, whichever is known).
    pub ended_at_us: i64,
    pub whole: Segment,
    /// Strokes the recording cut off or that span a pause, left out of `whole`.
    pub partial_strokes: u32,
    /// From [`DECOUPLING_WARMUP_S`] on, split at the midpoint.
    pub first_half: Option<Segment>,
    pub second_half: Option<Segment>,
    /// EF drop from first to second half, percent. Under 5% means aerobic
    /// fitness covers this duration.
    pub decoupling_pct: Option<f64>,
    pub last_10_min: Option<Segment>,
    /// Strokes rowed at 19.5–20.5 spm, by their own timing.
    pub rate_20: Option<Segment>,
    /// Median over strokes.
    pub drag_factor: Option<u8>,
    /// Mean drive time and recovery/drive: the handle's, by the camera, when
    /// it saw most strokes (`handle_timing`); else the PM5's, whose drive is
    /// only the flywheel speeding up.
    pub drive_ms: Option<f64>,
    pub ratio: Option<f64>,
    #[serde(default)]
    pub handle_timing: bool,
    pub peak_n: Option<f64>,
    /// Readings from first catch to end; zones and TRIMP need a profile.
    pub heart: Option<HeartStats>,
    pub profile: Option<HeartProfile>,
    /// How the camera saw the session, when there was one.
    pub camera: Option<CameraView>,
    /// How often each fault showed up (camera faults only with the camera).
    pub faults: Vec<FaultRate>,
}

/// `None` when there are no strokes.
pub fn summarize(strokes: &[StrokeRecord], hearts: &[(i64, u16)], profile: Option<&HeartProfile>) -> Option<Summary> {
    let first = strokes.first()?;
    let last = strokes.last()?;
    let start = first.t_catch_us;
    let end = last.t_next_catch_us.or(last.t_finish_us).unwrap_or(last.t_catch_us);
    let whole = segment(strokes);

    let in_window = |from_s: f64, to_s: f64| {
        // `as` saturates, so an infinite `to_s` becomes i64::MAX; add saturating too.
        let (a, b) = (
            start.saturating_add((from_s * 1e6) as i64),
            start.saturating_add((to_s * 1e6) as i64),
        );
        segment(strokes.iter().filter(move |s| s.t_catch_us >= a && s.t_catch_us < b))
    };
    let span_s = (end - start) as f64 / 1e6;

    let (first_half, second_half, decoupling_pct) = if span_s - DECOUPLING_WARMUP_S >= DECOUPLING_MIN_S {
        let mid = (DECOUPLING_WARMUP_S + span_s) / 2.0;
        let a = in_window(DECOUPLING_WARMUP_S, mid);
        let b = in_window(mid, f64::INFINITY);
        let pct = a.ef.zip(b.ef).map(|(e1, e2)| (e1 - e2) / e1 * 100.0);
        (Some(a), Some(b), pct)
    } else {
        (None, None, None)
    };

    let last_10_min = (span_s >= LAST_10_MIN_SESSION_S).then(|| in_window(span_s - 600.0, f64::INFINITY));

    let at_20 = segment(
        strokes
            .iter()
            .filter(|s| s.rate().is_some_and(|r| (RATE_20.0..=RATE_20.1).contains(&r))),
    );
    let rate_20 = (at_20.seconds >= RATE_20_MIN_S).then_some(at_20);

    let mean = |xs: Vec<f64>| (!xs.is_empty()).then(|| xs.iter().sum::<f64>() / xs.len() as f64);
    let complete: Vec<_> = strokes.iter().filter(|s| rowing(s).is_some()).collect();
    let mut dfs: Vec<u8> = complete.iter().filter_map(|s| s.drag_factor).collect();
    dfs.sort_unstable();

    let window: Vec<_> = hearts.iter().copied().filter(|&(t, _)| t >= start && t <= end).collect();
    let timed: Vec<(f64, f64)> = complete.iter().filter_map(|s| handle_timing(s, s.pose.as_ref())).collect();
    let by_camera = !timed.is_empty() && timed.len() * 2 >= complete.len();
    Some(Summary {
        analysis_version: ANALYSIS_VERSION,
        started_at_us: start,
        ended_at_us: end,
        partial_strokes: strokes.len() as u32 - whole.strokes,
        whole,
        first_half,
        second_half,
        decoupling_pct,
        last_10_min,
        rate_20,
        drag_factor: dfs.get(dfs.len() / 2).copied(),
        drive_ms: if by_camera {
            mean(timed.iter().map(|t| t.0).collect())
        } else {
            mean(complete.iter().filter_map(|s| s.drive_ms).collect())
        },
        ratio: if by_camera {
            mean(timed.iter().map(|t| t.1 / t.0).collect())
        } else {
            mean(complete.iter().filter_map(|s| s.ratio).collect())
        },
        handle_timing: by_camera,
        peak_n: mean(complete.iter().filter_map(|s| s.peak_n).collect()),
        heart: heart_stats(&window, profile),
        profile: profile.copied(),
        camera: None,
        faults: fault_rates(strokes),
    })
}

/// Strokes this soon after the first catch are still starting up, not
/// representative of the piece.
pub const START_UP_S: f64 = 30.0;

fn settled(strokes: &[StrokeRecord]) -> impl Iterator<Item = &StrokeRecord> {
    let start = strokes.first().map_or(0, |s| s.t_catch_us);
    strokes
        .iter()
        .filter(move |s| rowing(s).is_some() && (s.t_catch_us - start) as f64 / 1e6 >= START_UP_S)
}

/// Point-by-point mean of every settled stroke's resampled force curve.
pub fn mean_curve(strokes: &[StrokeRecord]) -> Vec<f64> {
    let mut sum = vec![0.0; CURVE_POINTS];
    let mut n = 0;
    for s in settled(strokes).filter(|s| s.force_n.len() == CURVE_POINTS) {
        sum.iter_mut().zip(&s.force_n).for_each(|(a, b)| *a += b);
        n += 1;
    }
    if n == 0 {
        return Vec::new();
    }
    sum.into_iter().map(|v| v / n as f64).collect()
}

/// Indices of the settled strokes with the most and the least work.
pub fn work_extremes(strokes: &[StrokeRecord]) -> Option<(u32, u32)> {
    let mut with_work = settled(strokes).filter_map(|s| s.joules().map(|j| (s.index, j)));
    let first = with_work.next()?;
    let (max, min) = with_work.fold((first, first), |(max, min), x| {
        (if x.1 > max.1 { x } else { max }, if x.1 < min.1 { x } else { min })
    });
    Some((max.0, min.0))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::metrics::pace_from_watts;

    /// 2026-11-01 in Unix microseconds.
    pub(crate) const T0: i64 = 1_793_000_000_000_000;

    /// A steady row: `n` strokes of `cycle_s`, at `watts` by pace, with
    /// heart rate from `hr(i)`.
    pub(crate) fn steady(n: u32, cycle_s: f64, watts: f64, hr: impl Fn(u32) -> f64) -> Vec<StrokeRecord> {
        let metres = cycle_s / pace_from_watts(watts) * 500.0;
        (0..n)
            .map(|i| {
                // Real epoch timestamps, so offset arithmetic is tested at real magnitudes.
                let t = T0 + (i as f64 * cycle_s * 1e6) as i64;
                StrokeRecord {
                    index: i,
                    t_catch_us: t,
                    t_finish_us: Some(t + 1_000_000),
                    t_next_catch_us: Some(t + (cycle_s * 1e6) as i64),
                    drive_ms: Some(1000.0),
                    recovery_ms: Some(cycle_s * 1000.0 - 1000.0),
                    ratio: Some(cycle_s - 1.0),
                    watts: Some(watts.round() as u16),
                    spm: Some((60.0 / cycle_s).round() as u16),
                    distance_m: Some(metres),
                    drag_factor: Some(110),
                    hr_bpm: Some(hr(i)),
                    peak_n: Some(500.0),
                    force_n: vec![],
                    pose: None,
                    faults: vec![],
                }
            })
            .collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn segment_uses_average_pace() {
        let s = steady(10, 3.0, 83.0, |_| 140.0);
        let seg = segment(&s);
        assert_eq!(seg.strokes, 10);
        assert!(close(seg.seconds, 30.0));
        assert!(close(seg.watts.unwrap(), 83.0));
        assert!(close(seg.spm.unwrap(), 20.0));
        assert!(close(seg.ef.unwrap(), 83.0 / 140.0));
    }

    #[test]
    fn pauses_and_cut_off_strokes_are_left_out() {
        let mut s = steady(5, 3.0, 80.0, |_| 140.0);
        s[2].t_next_catch_us = Some(s[2].t_catch_us + 20_000_000); // stopped for 20 s
        s[4].t_next_catch_us = None; // recording ended mid-stroke
        let sum = summarize(&s, &[], None).unwrap();
        assert_eq!(sum.whole.strokes, 3);
        assert_eq!(sum.partial_strokes, 2);
        assert!(sum.heart.is_none());
    }

    #[test]
    fn steady_row_with_drift() {
        // 20 min at rate 20, 53 W; HR climbs 130 → 150 over the row.
        let n = 400;
        let s = steady(n, 3.0, 53.0, |i| 130.0 + 20.0 * i as f64 / n as f64);
        let sum = summarize(&s, &[], None).unwrap();
        assert!(close(sum.whole.watts.unwrap(), 53.0));
        let d = sum.decoupling_pct.unwrap();
        assert!(d > 3.0 && d < 8.0, "decoupling {d}");
        let last = sum.last_10_min.unwrap();
        assert_eq!(last.strokes, 200);
        assert!(last.hr.unwrap() > sum.whole.hr.unwrap());
        let r20 = sum.rate_20.unwrap();
        assert_eq!(r20.strokes, n);
        assert_eq!(sum.drag_factor, Some(110));
    }

    #[test]
    fn short_or_off_rate_rows_skip_the_long_metrics() {
        let s = steady(100, 2.5, 90.0, |_| 150.0); // 24 spm, ~4 min
        let sum = summarize(&s, &[], None).unwrap();
        assert!(sum.decoupling_pct.is_none());
        assert!(sum.last_10_min.is_none());
        assert!(sum.rate_20.is_none());
        assert!(summarize(&[], &[], None).is_none());
    }

    #[test]
    fn missing_distance_falls_back_to_stroke_power() {
        let mut s = steady(10, 3.0, 80.0, |_| 140.0);
        s[3].distance_m = None;
        s[4].watts = Some(100);
        let seg = segment(&s);
        assert_eq!(seg.distance_m, None);
        assert!(close(seg.watts.unwrap(), 82.0));
    }

    #[test]
    fn mean_curve_and_extremes_skip_the_start() {
        let mut s = steady(40, 3.0, 80.0, |_| 140.0);
        for (i, r) in s.iter_mut().enumerate() {
            r.force_n = vec![i as f64; CURVE_POINTS];
        }
        s[0].watts = Some(10); // start-up stroke: never "weakest"
        s[20].watts = Some(120);
        s[30].watts = Some(60);
        let mean = mean_curve(&s);
        // From 30 s on: strokes 10..=39.
        assert_eq!(mean.len(), CURVE_POINTS);
        assert!(close(mean[0], (10..=39).sum::<usize>() as f64 / 30.0));
        assert_eq!(work_extremes(&s), Some((20, 30)));
        assert_eq!(work_extremes(&s[..5]), None);
    }
}
