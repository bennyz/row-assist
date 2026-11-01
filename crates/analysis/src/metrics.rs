//! Rowing formulas.

pub const LBF_TO_N: f64 = 4.448_221_615_260_5;

/// Concept2's power model: `watts = 2.80 / (seconds per metre)^3`.
pub fn watts_from_pace(sec_per_500: f64) -> f64 {
    2.80 / (sec_per_500 / 500.0).powi(3)
}

/// Seconds per 500 m at a given power.
pub fn pace_from_watts(watts: f64) -> f64 {
    500.0 * (2.80 / watts).cbrt()
}

/// Work done per stroke. At a fixed rate, this only rises if strokes get better.
pub fn joules_per_stroke(watts: f64, spm: f64) -> Option<f64> {
    (spm > 0.0).then(|| watts * 60.0 / spm)
}

pub fn metres_per_stroke(distance_m: f64, strokes: f64) -> Option<f64> {
    (strokes > 0.0).then(|| distance_m / strokes)
}

/// Efficiency factor: watts per heartbeat-per-minute. Compare only between
/// rows at a similar rate and effort.
pub fn efficiency_factor(watts: f64, hr_bpm: f64) -> Option<f64> {
    (hr_bpm > 0.0).then(|| watts / hr_bpm)
}

/// `m:ss.t` for a pace in seconds.
pub fn format_pace(sec: f64) -> String {
    let tenths = (sec * 10.0).round() as i64;
    format!("{}:{:02}.{}", tenths / 600, (tenths / 10) % 60, tenths % 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn pace_and_watts_match_concept2_tables() {
        assert!(close(watts_from_pace(120.0), 202.5, 0.1)); // 2:00 → 202.5 W
        assert!(close(watts_from_pace(187.5), 53.1, 0.1)); // 3:07.5 → 53 W (log, 2025-10-24)
        assert!(close(pace_from_watts(100.0), 151.8, 0.1)); // 100 W → 2:31.8
        assert!(close(pace_from_watts(watts_from_pace(161.8)), 161.8, 1e-9));
    }

    #[test]
    fn per_stroke_metrics() {
        assert!(close(joules_per_stroke(83.0, 20.0).unwrap(), 249.0, 1e-9));
        assert!(close(metres_per_stroke(3164.0, 347.0).unwrap(), 9.118, 0.001));
        assert_eq!(joules_per_stroke(80.0, 0.0), None);
        assert_eq!(efficiency_factor(50.0, 0.0), None);
    }

    #[test]
    fn formats_pace() {
        assert_eq!(format_pace(161.8), "2:41.8");
        assert_eq!(format_pace(59.96), "1:00.0");
    }
}
