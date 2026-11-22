//! Workout presets for `record` and `watch`, and target parsing.

use analysis::cues::Targets;
use analysis::workout::{Kind, Workout, mmss};
use clap::ValueEnum;

/// The workouts, by their command-line names.
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
    /// Rate steps: 4 × 5 min at rate 20 / 22 / 24 / 22 (set the intervals on the PM5)
    RateSteps,
    /// Norwegian 4×4: 4 × 4 min hard, 3 min easy between (set the intervals on the PM5)
    FourByFour,
}

impl Plan {
    pub fn kind(self) -> Kind {
        match self {
            Self::Reference => Kind::Reference,
            Self::Easy => Kind::Easy,
            Self::Steady => Kind::Steady,
            Self::Rate20 => Kind::Rate20Test,
            Self::RateSteps => Kind::RateSteps,
            Self::FourByFour => Kind::FourByFour,
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
    parts.join(", ")
}

/// "rate 19–21, 48–58 W, 20 min", or "4 × 4:00 (3:00 rest): rate 24–28,
/// HR under 175"; intervals with different rates list them.
pub fn describe_workout(w: &Workout) -> String {
    match w {
        Workout::Piece { minutes, targets } => {
            let mut parts: Vec<String> = Some(describe_targets(targets)).filter(|s| !s.is_empty()).into_iter().collect();
            parts.extend(minutes.map(|m| format!("{} min", num(m))));
            parts.join(", ")
        }
        Workout::Intervals { work_s, rest_s, work } => {
            let head = format!("{} × {} ({} rest)", work.len(), mmss(*work_s), mmss(*rest_s));
            let body = if work.windows(2).all(|p| p[0] == p[1]) {
                work.first().map(describe_targets).unwrap_or_default()
            } else {
                let rates: Vec<String> = work.iter().filter_map(|t| t.rate).map(|(lo, hi)| num((lo + hi) / 2.0)).collect();
                format!("rate {}", rates.join(", "))
            };
            if body.is_empty() { head } else { format!("{head}: {body}") }
        }
    }
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
    fn descriptions() {
        assert_eq!(describe_workout(&Kind::Reference.workout(None, None)), "rate 19–21, 48–58 W, 20 min");
        let h = analysis::zones::HeartProfile::new(60.0, 184.0, None).unwrap();
        assert_eq!(
            describe_workout(&Kind::FourByFour.workout(Some(&h), None)),
            "4 × 4:00 (3:00 rest): rate 20–26, HR under 175"
        );
        assert_eq!(describe_workout(&Kind::RateSteps.workout(None, None)), "4 × 5:00 (2:00 rest): rate 20, 22, 24, 22");
    }

    /// Every workout has a command-line name, and it's the same workout.
    #[test]
    fn every_kind_but_the_minimum_session_has_a_preset() {
        for kind in Kind::ALL {
            let preset = Plan::value_variants().iter().find(|p| p.kind() == kind);
            assert_eq!(preset.is_some(), kind != Kind::Minimum, "{kind:?}");
        }
    }
}
