//! Readiness from Apple Watch recovery data (DESIGN.md §7): explicit,
//! tunable rules, not a black box.
//!
//! - HRV: the 7-day mean of ln(HRV) against the normal band of the 60 days
//!   before it (mean ± 1 SD of the daily values). Day-to-day HRV is noisy;
//!   the weekly trend against your own band is what means something.
//! - Resting HR: 7-day mean against the 30 days before it.
//! - Sleep: last night.

use serde::{Deserialize, Serialize};

/// One day of recovery data. `day` counts days since 1970-01-01, local.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HealthDay {
    pub day: i64,
    pub hrv_ms: Option<f64>,
    pub resting_hr: Option<f64>,
    pub sleep_h: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Not enough data for a baseline yet.
    Unknown,
    Green,
    Amber,
    Red,
}

impl Level {
    pub fn advice(self) -> &'static str {
        match self {
            Self::Unknown => "not enough Watch data yet: row as planned",
            Self::Green => "row as planned",
            Self::Amber => "easy only: no intensity today",
            Self::Red => "minimum session: 15–20 min easy, or rest",
        }
    }
}

pub const WEEK: i64 = 7;
pub const HRV_BASELINE_DAYS: i64 = 60;
pub const RHR_BASELINE_DAYS: i64 = 30;
/// A baseline needs this many days with data.
pub const MIN_BASELINE_DAYS: usize = 14;
/// Days of resting HR the zones' HRrest is the median of.
pub const ZONE_REST_DAYS: i64 = 14;
/// The weekly ln(HRV) more than this many SDs below the band is a red flag.
const HRV_RED_SD: f64 = 2.0;
const RHR_AMBER_BPM: f64 = 5.0;
const RHR_RED_BPM: f64 = 8.0;
const SLEEP_AMBER_H: f64 = 6.0;
const SLEEP_RED_H: f64 = 5.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Readiness {
    pub level: Level,
    /// Why, one line per signal that isn't fine.
    pub reasons: Vec<String>,
    /// 7-day mean of ln(HRV) and the normal band it's judged against.
    pub hrv_week_ln: Option<f64>,
    pub hrv_band_ln: Option<(f64, f64)>,
    pub rhr_week: Option<f64>,
    pub rhr_baseline: Option<f64>,
    pub sleep_last_h: Option<f64>,
}

fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

fn sd(v: &[f64]) -> Option<f64> {
    let m = mean(v)?;
    (v.len() > 1).then(|| (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt())
}

/// Values of `pick` for days in `[from, to)`.
fn window(days: &[HealthDay], from: i64, to: i64, pick: fn(&HealthDay) -> Option<f64>) -> Vec<f64> {
    days.iter().filter(|d| d.day >= from && d.day < to).filter_map(pick).collect()
}

/// Readiness for `today` (its own night counts as last night).
pub fn readiness(days: &[HealthDay], today: i64) -> Readiness {
    let week_from = today - WEEK + 1;
    let ln_hrv = |d: &HealthDay| d.hrv_ms.filter(|&v| v > 0.0).map(f64::ln);
    let hrv_week = window(days, week_from, today + 1, ln_hrv);
    let hrv_base = window(days, week_from - HRV_BASELINE_DAYS, week_from, ln_hrv);
    let rhr_week = window(days, week_from, today + 1, |d| d.resting_hr);
    let rhr_base = window(days, week_from - RHR_BASELINE_DAYS, week_from, |d| d.resting_hr);
    let sleep_last_h = days.iter().find(|d| d.day == today).and_then(|d| d.sleep_h);

    let mut flags: Vec<(u8, String)> = Vec::new();
    let mut r = Readiness {
        level: Level::Unknown,
        reasons: Vec::new(),
        hrv_week_ln: mean(&hrv_week),
        hrv_band_ln: None,
        rhr_week: mean(&rhr_week),
        rhr_baseline: None,
        sleep_last_h,
    };

    let mut have_baseline = false;
    if hrv_base.len() >= MIN_BASELINE_DAYS
        && let (Some(m), Some(s), Some(w)) = (mean(&hrv_base), sd(&hrv_base), r.hrv_week_ln)
    {
        have_baseline = true;
        r.hrv_band_ln = Some((m - s, m + s));
        if w < m - HRV_RED_SD * s {
            flags.push((2, format!("HRV well below your normal ({:.0} ms vs {:.0}–{:.0})", w.exp(), (m - s).exp(), (m + s).exp())));
        } else if w < m - s {
            flags.push((1, format!("HRV below your normal ({:.0} ms vs {:.0}–{:.0})", w.exp(), (m - s).exp(), (m + s).exp())));
        }
    }
    if rhr_base.len() >= MIN_BASELINE_DAYS
        && let (Some(b), Some(w)) = (mean(&rhr_base), r.rhr_week)
    {
        have_baseline = true;
        r.rhr_baseline = Some(b);
        let up = w - b;
        if up > RHR_RED_BPM {
            flags.push((2, format!("resting HR {up:+.0} over your baseline ({w:.0} vs {b:.0})")));
        } else if up > RHR_AMBER_BPM {
            flags.push((1, format!("resting HR {up:+.0} over your baseline ({w:.0} vs {b:.0})")));
        }
    }
    if let Some(h) = sleep_last_h {
        if h < SLEEP_RED_H {
            flags.push((2, format!("{h:.1} h sleep last night")));
        } else if h < SLEEP_AMBER_H {
            flags.push((1, format!("{h:.1} h sleep last night")));
        }
    }

    let reds = flags.iter().filter(|f| f.0 == 2).count();
    let ambers = flags.iter().filter(|f| f.0 == 1).count();
    r.level = if reds > 0 || ambers >= 2 {
        Level::Red
    } else if ambers == 1 {
        Level::Amber
    } else if have_baseline {
        Level::Green
    } else {
        Level::Unknown
    };
    r.reasons = flags.into_iter().map(|f| f.1).collect();
    r
}

/// HRrest for the zones: the median Watch resting HR over the last two weeks.
pub fn zone_rest(days: &[HealthDay], today: i64) -> Option<f64> {
    let mut v = window(days, today - ZONE_REST_DAYS + 1, today + 1, |d| d.resting_hr);
    if v.len() < WEEK as usize {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[v.len() / 2])
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: i64 = 20_400;

    /// 90 days of steady recovery: HRV ~55 ms varying ~11% day to day (as
    /// Watch HRV does), resting HR ~55, 7.5 h sleep.
    fn steady() -> Vec<HealthDay> {
        (TODAY - 89..=TODAY)
            .map(|day| {
                let wobble = ((day * 7919) % 11) as f64 - 5.0; // -5..5, deterministic
                HealthDay {
                    day,
                    hrv_ms: Some(55.0 + 2.0 * wobble),
                    resting_hr: Some(55.0 + wobble / 5.0),
                    sleep_h: Some(7.5),
                }
            })
            .collect()
    }

    fn last_week(days: &mut [HealthDay], f: impl Fn(&mut HealthDay)) {
        for d in days.iter_mut().filter(|d| d.day > TODAY - WEEK) {
            f(d);
        }
    }

    #[test]
    fn steady_is_green() {
        let r = readiness(&steady(), TODAY);
        assert_eq!(r.level, Level::Green, "{:?}", r.reasons);
        assert!(r.reasons.is_empty());
        let (lo, hi) = r.hrv_band_ln.unwrap();
        assert!(lo < 55f64.ln() && 55f64.ln() < hi);
    }

    #[test]
    fn a_suppressed_week_turns_amber_then_red() {
        let mut d = steady();
        last_week(&mut d, |x| x.hrv_ms = Some(48.0));
        let r = readiness(&d, TODAY);
        assert_eq!(r.level, Level::Amber, "{:?}", r.reasons);
        assert!(r.reasons[0].starts_with("HRV below"));

        // HRV down and resting HR up together: red.
        last_week(&mut d, |x| x.resting_hr = Some(61.0));
        assert_eq!(readiness(&d, TODAY).level, Level::Red);
    }

    #[test]
    fn short_sleep_counts_for_last_night_only() {
        let mut d = steady();
        d.last_mut().unwrap().sleep_h = Some(5.5);
        let r = readiness(&d, TODAY);
        assert_eq!(r.level, Level::Amber);
        assert_eq!(r.reasons, vec!["5.5 h sleep last night"]);
        d.last_mut().unwrap().sleep_h = Some(4.0);
        assert_eq!(readiness(&d, TODAY).level, Level::Red);
        // Yesterday's short night doesn't count today.
        let r = readiness(&d, TODAY + 1);
        assert!(!r.reasons.iter().any(|x| x.contains("sleep")));
    }

    #[test]
    fn a_new_watch_has_no_baseline() {
        let d: Vec<HealthDay> = steady().into_iter().filter(|x| x.day > TODAY - 10).collect();
        let r = readiness(&d, TODAY);
        assert_eq!(r.level, Level::Unknown);
        assert_eq!(r.hrv_band_ln, None);
        assert!(readiness(&[], TODAY).reasons.is_empty());
    }

    #[test]
    fn zone_rest_is_the_two_week_median() {
        let mut d = steady();
        last_week(&mut d, |x| x.resting_hr = Some(52.0));
        let rest = zone_rest(&d, TODAY).unwrap();
        assert!((52.0..=56.0).contains(&rest), "{rest}");
        assert_eq!(zone_rest(&d[..5], TODAY), None, "needs a week of data");
    }
}
