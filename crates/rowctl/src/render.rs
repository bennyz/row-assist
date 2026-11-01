//! Terminal formatting shared by the commands.

use analysis::metrics::LBF_TO_N;
use analysis::strokes::{DriveCurve, StrokeRecord};
use analysis::zones::HeartProfile;

const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Resample to `width` columns (max per bucket, so the peak survives) and
/// scale to the curve's own peak.
pub fn sparkline<T: Copy + Into<f64>>(points: &[T], width: usize) -> String {
    let vals: Vec<f64> = points.iter().map(|&p| p.into()).collect();
    let peak = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if vals.is_empty() {
        return String::new();
    }
    let cols = vals.len().min(width);
    if peak <= 0.0 {
        return BARS[0].to_string().repeat(cols);
    }
    (0..cols)
        .map(|c| {
            let lo = c * vals.len() / cols;
            let hi = ((c + 1) * vals.len() / cols).max(lo + 1);
            let v = vals[lo..hi].iter().copied().fold(0.0, f64::max);
            BARS[((v / peak * (BARS.len() - 1) as f64).round() as usize).min(BARS.len() - 1)]
        })
        .collect()
}

pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn opt(v: Option<f64>, fmt: impl Fn(f64) -> String) -> String {
    v.map(fmt).unwrap_or_else(|| "–".into())
}

/// One line per stroke.
pub fn stroke_line(c: &DriveCurve, start_us: i64) -> String {
    let t = (c.drive_start_us - start_us) as f64 / 1e6;
    let peak = c.peak_lbf();
    format!(
        "#{:<4} {}  drive {:>5}  rec {:>5}  ratio {:>4}  peak {:>4} lbf ({:>4} N)  {:>3} pts  {}",
        c.index + 1,
        clock(t),
        opt(c.drive_ms(), |ms| format!("{:.2}s", ms / 1000.0)),
        opt(c.recovery_ms(), |ms| format!("{:.2}s", ms / 1000.0)),
        opt(c.ratio(), |r| format!("{r:.1}")),
        peak.map(|p| p.to_string()).unwrap_or_else(|| "–".into()),
        peak.map(|p| format!("{:.0}", p as f64 * LBF_TO_N)).unwrap_or_else(|| "–".into()),
        c.points.len(),
        sparkline(&c.points, 32),
    )
}

/// One line per stroke record, for the live view.
pub fn record_line(r: &StrokeRecord, start_us: i64, heart: Option<&HeartProfile>) -> String {
    let t = (r.t_catch_us - start_us) as f64 / 1e6;
    let hr = match (r.hr_bpm, heart) {
        (Some(b), Some(h)) => format!("HR {b:>3.0} Z{}", h.zone(b)),
        (Some(b), None) => format!("HR {b:>3.0}"),
        (None, _) => "HR   –".into(),
    };
    format!(
        "#{:<4} {}  drive {:>5}  ratio {:>4}  {:>3} W {:>2} spm {:>5} m  {}  peak {:>4} N  {}",
        r.index + 1,
        clock(t),
        opt(r.drive_ms, |ms| format!("{:.2}s", ms / 1000.0)),
        opt(r.ratio, |x| format!("{x:.1}")),
        r.watts.map(|w| w.to_string()).unwrap_or_else(|| "–".into()),
        r.spm.map(|w| w.to_string()).unwrap_or_else(|| "–".into()),
        opt(r.distance_m, |d| format!("{d:.1}")),
        hr,
        opt(r.peak_n, |p| format!("{p:.0}")),
        sparkline(&r.force_n, 25),
    ) + &faults(r)
}

fn faults(r: &StrokeRecord) -> String {
    if r.faults.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = r.faults.iter().map(|h| h.fault.label()).collect();
    format!("  [{}]", names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparkline_keeps_the_peak() {
        let s = sparkline(&[0, 50, 100, 50, 0], 5);
        assert_eq!(s, "▁▅█▅▁");
        // Downsampling 100 points to 10 columns still shows the top bar.
        let curve: Vec<u16> = (0..100).map(|i| if i == 37 { 200 } else { 10 }).collect();
        assert!(sparkline(&curve, 10).contains('█'));
        assert_eq!(sparkline(&curve, 10).chars().count(), 10);
    }

    #[test]
    fn sparkline_handles_edges() {
        assert_eq!(sparkline::<u16>(&[], 10), "");
        assert_eq!(sparkline(&[0, 0], 10), "▁▁");
    }

    #[test]
    fn clock_formats() {
        assert_eq!(clock(0.0), "00:00");
        assert_eq!(clock(125.9), "02:05");
    }
}
