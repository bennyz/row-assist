//! Heart-rate zones (Karvonen), training load (Banister TRIMP) and time in zone.

use model::{Event, EventKind};
use serde::{Deserialize, Serialize};

/// Zone floors as fractions of heart-rate reserve. Zone 1 starts at 50%;
/// below that is zone 0.
pub const ZONE_FLOORS: [f64; 5] = [0.50, 0.60, 0.70, 0.80, 0.90];

/// A gap between readings longer than this is a dropout, not time at that rate.
const MAX_SAMPLE_GAP_US: i64 = 5_000_000;
/// Window for the sustained peak, which ignores single-beat spikes.
const PEAK_WINDOW_US: i64 = 10_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sex {
    Male,
    Female,
}

impl Sex {
    /// Banister's TRIMP weighting exponent.
    fn trimp_b(sex: Option<Self>) -> f64 {
        match sex {
            Some(Self::Female) => 1.67,
            // The original (male) coefficient when unset.
            Some(Self::Male) | None => 1.92,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HeartProfile {
    pub rest: f64,
    pub max: f64,
    pub sex: Option<Sex>,
}

/// Where each [`HeartProfile`] value came from, for showing the rower.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Set,
    /// Resting HR from the Apple Watch (two-week median).
    Watch,
    /// HRmax from Tanaka's `208 − 0.7 × age`; rest from a typical value.
    Estimated,
}

/// Assumed resting heart rate until one is set (or comes from the Watch).
pub const DEFAULT_REST: f64 = 60.0;
/// HRmax when neither a value nor an age is known.
pub const DEFAULT_MAX: f64 = 190.0;

impl HeartProfile {
    /// `None` if the reserve is implausibly small (max must exceed rest by 40).
    pub fn new(rest: f64, max: f64, sex: Option<Sex>) -> Option<Self> {
        (rest > 0.0 && max - rest >= 40.0).then_some(Self { rest, max, sex })
    }

    /// Fill gaps with estimates. Returns the profile and the source of
    /// (rest, max).
    pub fn resolve(
        rest: Option<f64>,
        max: Option<f64>,
        age: Option<f64>,
        sex: Option<Sex>,
    ) -> Option<(Self, Source, Source)> {
        let (rest, rest_src) = rest.map_or((DEFAULT_REST, Source::Estimated), |r| (r, Source::Set));
        let (max, max_src) = match (max, age) {
            (Some(m), _) => (m, Source::Set),
            (None, Some(age)) => (208.0 - 0.7 * age, Source::Estimated),
            (None, None) => (DEFAULT_MAX, Source::Estimated),
        };
        Self::new(rest, max, sex).map(|p| (p, rest_src, max_src))
    }

    pub fn reserve_fraction(&self, bpm: f64) -> f64 {
        (bpm - self.rest) / (self.max - self.rest)
    }

    pub fn bpm_at(&self, fraction: f64) -> f64 {
        self.rest + fraction * (self.max - self.rest)
    }

    /// 0 (below zone 1) to 5.
    pub fn zone(&self, bpm: f64) -> u8 {
        let f = self.reserve_fraction(bpm);
        ZONE_FLOORS.iter().filter(|&&floor| f >= floor).count() as u8
    }

    /// `[low, high)` bpm for zones 1 to 5; zone 5's top is HRmax.
    pub fn zone_bpm(&self, zone: u8) -> Option<(f64, f64)> {
        let i = (zone as usize).checked_sub(1).filter(|&i| i < ZONE_FLOORS.len())?;
        let hi = ZONE_FLOORS.get(i + 1).copied().unwrap_or(1.0);
        Some((self.bpm_at(ZONE_FLOORS[i]), self.bpm_at(hi)))
    }
}

/// Heart readings worth using: `(t_us, bpm)`, skipping zeros and lost contact.
pub fn heart_samples<'a>(events: impl IntoIterator<Item = &'a Event>) -> Vec<(i64, u16)> {
    events
        .into_iter()
        .filter_map(|ev| match &ev.kind {
            EventKind::Heart(h) if h.bpm > 0 && h.contact != Some(false) => Some((ev.t_us, h.bpm)),
            _ => None,
        })
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HeartStats {
    /// Seconds covered by readings (dropouts excluded).
    pub seconds: f64,
    pub mean_bpm: f64,
    pub max_bpm: u16,
    /// Highest 10-second mean: a max that one spurious beat can't set.
    pub peak_10s_bpm: Option<f64>,
    /// Seconds in zones 0..=5. Empty without a profile.
    #[serde(default)]
    pub zone_s: Vec<f64>,
    pub trimp: Option<f64>,
}

/// Each reading counts until the next one (capped, so a dropout adds nothing);
/// the last counts for one second.
pub fn heart_stats(samples: &[(i64, u16)], profile: Option<&HeartProfile>) -> Option<HeartStats> {
    let &(_, first) = samples.first()?;
    let mut stats = HeartStats {
        max_bpm: first,
        zone_s: if profile.is_some() { vec![0.0; 6] } else { Vec::new() },
        ..Default::default()
    };
    let mut weighted = 0.0;
    let mut trimp = 0.0;
    for (i, &(t, bpm)) in samples.iter().enumerate() {
        let dt_us = samples.get(i + 1).map_or(1_000_000, |&(next, _)| next - t);
        let dt = if dt_us > MAX_SAMPLE_GAP_US { 0.0 } else { dt_us.max(0) as f64 / 1e6 };
        stats.seconds += dt;
        weighted += bpm as f64 * dt;
        stats.max_bpm = stats.max_bpm.max(bpm);
        if let Some(p) = profile {
            stats.zone_s[p.zone(bpm as f64) as usize] += dt;
            let hrr = p.reserve_fraction(bpm as f64).clamp(0.0, 1.0);
            trimp += dt / 60.0 * hrr * 0.64 * (Sex::trimp_b(p.sex) * hrr).exp();
        }
    }
    if stats.seconds <= 0.0 {
        return None;
    }
    stats.mean_bpm = weighted / stats.seconds;
    stats.peak_10s_bpm = peak_mean(samples, PEAK_WINDOW_US);
    stats.trimp = profile.map(|_| trimp);
    Some(stats)
}

/// Highest mean over any window spanning at least 80% of `window_us`.
fn peak_mean(samples: &[(i64, u16)], window_us: i64) -> Option<f64> {
    let mut best: Option<f64> = None;
    let mut sum = 0.0;
    let mut lo = 0;
    for hi in 0..samples.len() {
        sum += samples[hi].1 as f64;
        while samples[hi].0 - samples[lo].0 > window_us {
            sum -= samples[lo].1 as f64;
            lo += 1;
        }
        if (samples[hi].0 - samples[lo].0) * 5 >= window_us * 4 {
            let mean = sum / (hi - lo + 1) as f64;
            best = Some(best.map_or(mean, |b: f64| b.max(mean)));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> HeartProfile {
        HeartProfile::new(60.0, 190.0, Some(Sex::Male)).unwrap()
    }

    #[test]
    fn karvonen_zones() {
        let p = profile();
        // Reserve 130: zone 2 is 60–70% → 138–151 bpm.
        assert_eq!(p.zone_bpm(2), Some((138.0, 151.0)));
        assert_eq!(p.zone(137.9), 1);
        assert_eq!(p.zone(138.0), 2);
        assert_eq!(p.zone(100.0), 0);
        assert_eq!(p.zone(189.0), 5);
        assert_eq!(p.zone_bpm(5), Some((177.0, 190.0)));
        assert_eq!(p.zone_bpm(0), None);
        assert_eq!(p.zone_bpm(6), None);
    }

    #[test]
    fn resolve_fills_gaps() {
        let (p, rest, max) = HeartProfile::resolve(None, None, Some(35.0), None).unwrap();
        assert_eq!((p.rest, p.max), (60.0, 183.5));
        assert_eq!((rest, max), (Source::Estimated, Source::Estimated));
        let (p, _, max) = HeartProfile::resolve(Some(52.0), Some(188.0), Some(35.0), None).unwrap();
        assert_eq!((p.rest, p.max, max), (52.0, 188.0, Source::Set));
        assert!(HeartProfile::resolve(Some(150.0), Some(160.0), None, None).is_none());
    }

    #[test]
    fn stats_skip_dropouts() {
        // 140 for 10 s at 1 Hz, a 30 s dropout, then 150 for 5 s.
        let mut s: Vec<(i64, u16)> = (0..10).map(|i| (i * 1_000_000, 140)).collect();
        s.extend((0..5).map(|i| (40_000_000 + i * 1_000_000, 150)));
        let st = heart_stats(&s, Some(&profile())).unwrap();
        // 9 s + (gap dropped) + 5 s (4 gaps + 1 s for the last).
        assert_eq!(st.seconds, 14.0);
        assert!((st.mean_bpm - (140.0 * 9.0 + 150.0 * 5.0) / 14.0).abs() < 1e-9);
        assert_eq!(st.max_bpm, 150);
        assert_eq!(st.zone_s[2], 14.0, "both rates are zone 2");
        assert!(st.trimp.unwrap() > 0.0);
    }

    #[test]
    fn trimp_matches_banister() {
        // 60 min at 50% reserve, male: 60 × 0.5 × 0.64 × e^(0.96) ≈ 50.1.
        let p = profile();
        let bpm = p.bpm_at(0.5) as u16; // 125
        let s: Vec<(i64, u16)> = (0..3600).map(|i| (i * 1_000_000, bpm)).collect();
        let st = heart_stats(&s, Some(&p)).unwrap();
        assert!((st.trimp.unwrap() - 60.0 * 0.5 * 0.64 * 0.96f64.exp()).abs() < 0.05);
    }

    #[test]
    fn peak_ignores_a_single_spike() {
        let mut s: Vec<(i64, u16)> = (0..30).map(|i| (i * 1_000_000, 150)).collect();
        s[15].1 = 220;
        let st = heart_stats(&s, None).unwrap();
        assert_eq!(st.max_bpm, 220);
        let peak = st.peak_10s_bpm.unwrap();
        assert!(peak < 160.0, "peak {peak}");
        assert!(st.zone_s.is_empty() && st.trimp.is_none());
    }
}
