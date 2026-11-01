//! Workout presets for `record` and `watch`, and target parsing.

use analysis::cues::Targets;
use analysis::zones::HeartProfile;
use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Plan {
    /// 20 min at rate 20, 48–58 W: the comeback-ramp reference row
    Reference,
    /// 20 min easy at rate 18–20, heart rate kept under zone 2's top
    Easy,
    /// 30 min steady in zone 2 at rate 18–20
    Steady,
    /// The benchmark: 20 min held at exactly rate 20, best steady power
    Rate20,
    /// Rate steps (set the intervals on the PM5): only time calls
    RateSteps,
}

impl Plan {
    /// Stored in the recording header.
    pub fn name(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Easy => "easy",
            Self::Steady => "steady",
            Self::Rate20 => "rate20",
            Self::RateSteps => "rate_steps",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Reference => "Reference row: 20 min at rate 20, about 53 W, same drag factor as every reference row",
            Self::Easy => "Easy row: 20–25 min at rate 18–20, heart rate in zone 2 or below",
            Self::Steady => "Steady row: 30–45 min in zone 2 at rate 18–20",
            Self::Rate20 => "Rate-20 test: 20 min at exactly 20 spm, as much steady power as you can hold",
            Self::RateSteps => "Rate steps: 4 × 5 min at rate 20 / 22 / 24 / 22 with 2 min easy between, set on the PM5",
        }
    }

    pub fn targets(self, heart: Option<&HeartProfile>) -> Targets {
        match self {
            Self::Reference => Targets {
                rate: Some((19.0, 21.0)),
                watts: Some((48.0, 58.0)),
                hr_max: None,
                minutes: Some(20.0),
            },
            Self::Easy => Targets {
                rate: Some((17.5, 20.5)),
                watts: None,
                hr_max: heart.and_then(|h| h.zone_bpm(2)).map(|(_, top)| top),
                minutes: Some(20.0),
            },
            Self::Steady => Targets {
                rate: Some((17.5, 20.5)),
                watts: None,
                hr_max: heart.and_then(|h| h.zone_bpm(2)).map(|(_, top)| top),
                minutes: Some(30.0),
            },
            Self::Rate20 => Targets {
                rate: Some((19.5, 20.5)),
                watts: None,
                hr_max: None,
                minutes: Some(20.0),
            },
            Self::RateSteps => Targets {
                minutes: Some(34.0),
                ..Default::default()
            },
        }
    }
}

/// `19-21`, or one number meaning ±1.
pub fn parse_rate(s: &str) -> Result<(f64, f64), String> {
    parse_range(s, 1.0)
}

/// `48-58`, or one number meaning ±5.
pub fn parse_watts(s: &str) -> Result<(f64, f64), String> {
    parse_range(s, 5.0)
}

/// `120-142`, or one number for a constant rate.
pub fn parse_bpm(s: &str) -> Result<(f64, f64), String> {
    parse_range(s, 0.0)
}

fn parse_range(s: &str, tolerance: f64) -> Result<(f64, f64), String> {
    let num = |v: &str| v.trim().parse::<f64>().map_err(|_| format!("not a number: {v:?}"));
    let (lo, hi) = match s.split_once('-') {
        Some((a, b)) => (num(a)?, num(b)?),
        None => {
            let v = num(s)?;
            (v - tolerance, v + tolerance)
        }
    };
    if lo > 0.0 && lo <= hi {
        Ok((lo, hi))
    } else {
        Err(format!("expected low-high with 0 < low <= high, got {s:?}"))
    }
}

/// `17.5`, or `21` for whole numbers.
pub fn num(v: f64) -> String {
    if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{v:.1}") }
}

pub fn describe_targets(t: &Targets) -> String {
    let mut parts = Vec::new();
    if let Some((lo, hi)) = t.rate {
        parts.push(format!("rate {}–{}", num(lo), num(hi)));
    }
    if let Some((lo, hi)) = t.watts {
        parts.push(format!("{}–{} W", num(lo), num(hi)));
    }
    if let Some(max) = t.hr_max {
        parts.push(format!("HR under {max:.0}"));
    }
    if let Some(m) = t.minutes {
        parts.push(format!("{} min", num(m)));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_rate("19-21"), Ok((19.0, 21.0)));
        assert_eq!(parse_rate("20"), Ok((19.0, 21.0)));
        assert_eq!(parse_watts("53"), Ok((48.0, 58.0)));
        assert_eq!(parse_watts(" 50 - 55 "), Ok((50.0, 55.0)));
        assert!(parse_rate("21-19").is_err());
        assert!(parse_rate("fast").is_err());
        assert!(parse_watts("0-5").is_err());
        assert_eq!(parse_bpm("130"), Ok((130.0, 130.0)));
    }

    #[test]
    fn easy_ceiling_is_top_of_zone_2() {
        let h = HeartProfile::new(60.0, 190.0, None).unwrap();
        assert_eq!(Plan::Easy.targets(Some(&h)).hr_max, Some(151.0));
        assert_eq!(Plan::Easy.targets(None).hr_max, None);
        assert_eq!(
            describe_targets(&Plan::Reference.targets(None)),
            "rate 19–21, 48–58 W, 20 min"
        );
    }
}
