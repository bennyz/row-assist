//! The long view (DESIGN.md §6, long term): load and form, watts at rate 20,
//! EF of comparable steady rows, consistency, faults over time, benchmarks,
//! and form under fatigue.

use serde::{Deserialize, Serialize};

use crate::strokes::StrokeRecord;
use crate::technique::{Fault, FaultRate};
use crate::zones::HeartProfile;

/// One row, from either source.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProgressRow {
    pub day: i64,
    pub minutes: f64,
    pub distance_m: Option<f64>,
    pub watts: Option<f64>,
    pub spm: Option<f64>,
    pub hr: Option<f64>,
    /// From heart readings (app); `None` for logbook rows, estimated from avg HR.
    pub trimp: Option<f64>,
    pub watts_at_20: Option<f64>,
    pub plan: Option<String>,
    pub faults: Vec<FaultRate>,
    /// Interval workout (logbook rest time > 0).
    pub intervals: bool,
    pub from_logbook: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LoadDay {
    pub day: i64,
    pub trimp: f64,
    /// 42-day exponential average of load.
    pub fitness: f64,
    /// 7-day.
    pub fatigue: f64,
    /// Fitness minus fatigue: negative while building, positive when fresh.
    pub form: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub day: i64,
    pub value: f64,
    /// Watts for EF points; minutes for watts-at-20 points.
    pub context: f64,
    pub from_logbook: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Week {
    /// Monday.
    pub start_day: i64,
    pub rows: u32,
    pub minutes: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FaultPoint {
    pub day: i64,
    pub fault: Fault,
    pub rate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Benchmark {
    pub day: i64,
    /// `rate20` or `2k`.
    pub kind: String,
    /// Watts for rate20, seconds for 2k.
    pub value: f64,
}

/// A fault's rate early and late in sessions: does form hold as you tire?
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Fatigue {
    pub fault: Fault,
    pub first_half: f64,
    pub second_half: f64,
    pub sessions: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub today: i64,
    pub load: Vec<LoadDay>,
    pub watts_at_20: Vec<Point>,
    pub ef: Vec<Point>,
    /// The power band EF points are limited to.
    pub ef_watts: (f64, f64),
    pub weeks: Vec<Week>,
    /// Consecutive weeks with at least 3 rows, up to the last full week (or this one).
    pub streak_weeks: u32,
    pub faults: Vec<FaultPoint>,
    pub benchmarks: Vec<Benchmark>,
    pub fatigue: Vec<Fatigue>,
}

pub const FITNESS_DAYS: f64 = 42.0;
pub const FATIGUE_DAYS: f64 = 7.0;
/// EF is only comparable at similar power: the band around the reference row.
pub const EF_WATTS: (f64, f64) = (43.0, 63.0);
pub const STEADY_MIN_MINUTES: f64 = 15.0;
pub const STEADY_SPM: (f64, f64) = (16.0, 22.0);
pub const WEEK_TARGET: u32 = 3;
/// Logbook pieces at this rate count toward watts at 20 (its rate is a whole number).
const LOGBOOK_RATE_20: f64 = 20.0;
const RATE_20_MIN_MINUTES: f64 = 10.0;
/// Form under fatigue looks at the last this-many camera sessions.
const FATIGUE_SESSIONS: usize = 10;

/// Monday of the week `day` is in (day 0, 1970-01-01, was a Thursday).
pub fn week_start(day: i64) -> i64 {
    (day + 3).div_euclid(7) * 7 - 3
}

/// Banister TRIMP from an average heart rate: rougher than from every beat.
pub fn estimate_trimp(minutes: f64, hr: f64, p: &HeartProfile) -> f64 {
    let hrr = p.reserve_fraction(hr).clamp(0.0, 1.0);
    let b = if p.sex == Some(crate::zones::Sex::Female) { 1.67 } else { 1.92 };
    minutes * hrr * 0.64 * (b * hrr).exp()
}

/// Each fault's rate in the first and second half (by time) of one session.
pub fn fault_halves(strokes: &[StrokeRecord]) -> Vec<(Fault, f64, f64)> {
    let judged: Vec<&StrokeRecord> = strokes.iter().filter(|s| s.pose.is_some() && s.t_next_catch_us.is_some()).collect();
    let (Some(a), Some(b)) = (judged.first(), judged.last()) else {
        return Vec::new();
    };
    let mid = (a.t_catch_us + b.t_catch_us) / 2;
    let (first, second): (Vec<&&StrokeRecord>, Vec<&&StrokeRecord>) = judged.iter().partition(|s| s.t_catch_us < mid);
    let rate = |half: &[&&StrokeRecord], f: Fault| {
        half.iter().filter(|s| s.faults.iter().any(|h| h.fault == f)).count() as f64 / half.len().max(1) as f64
    };
    Fault::ALL
        .into_iter()
        .filter(|f| f.needs_camera())
        .map(|f| (f, rate(&first, f), rate(&second, f)))
        .collect()
}

/// `halves`: [`fault_halves`] of recent camera sessions, newest last.
pub fn progress(rows: &[ProgressRow], halves: &[Vec<(Fault, f64, f64)>], profile: Option<&HeartProfile>, today: i64) -> Progress {
    let mut rows: Vec<&ProgressRow> = rows.iter().filter(|r| r.day <= today).collect();
    rows.sort_by_key(|r| r.day);

    let load = match rows.first() {
        Some(first) => {
            let mut daily = vec![0.0; (today - first.day + 1) as usize];
            for r in &rows {
                let t = r.trimp.or_else(|| Some(estimate_trimp(r.minutes, r.hr?, profile?)));
                daily[(r.day - first.day) as usize] += t.unwrap_or(0.0);
            }
            let (mut fit, mut fat) = (0.0, 0.0);
            daily
                .iter()
                .enumerate()
                .map(|(i, &t)| {
                    fit += (t - fit) / FITNESS_DAYS;
                    fat += (t - fat) / FATIGUE_DAYS;
                    LoadDay {
                        day: first.day + i as i64,
                        trimp: t,
                        fitness: fit,
                        fatigue: fat,
                        form: fit - fat,
                    }
                })
                .collect()
        }
        None => Vec::new(),
    };

    let watts_at_20 = rows
        .iter()
        .filter_map(|r| {
            let w = r.watts_at_20.or_else(|| {
                (r.from_logbook && !r.intervals && r.spm == Some(LOGBOOK_RATE_20) && r.minutes >= RATE_20_MIN_MINUTES)
                    .then_some(r.watts?)
            })?;
            Some(Point { day: r.day, value: w, context: r.minutes, from_logbook: r.from_logbook })
        })
        .collect();

    let ef = rows
        .iter()
        .filter(|r| !r.intervals && r.minutes >= STEADY_MIN_MINUTES)
        .filter(|r| r.spm.is_some_and(|s| (STEADY_SPM.0..=STEADY_SPM.1).contains(&s)))
        .filter_map(|r| {
            let (w, hr) = (r.watts?, r.hr?);
            ((EF_WATTS.0..=EF_WATTS.1).contains(&w) && hr > 0.0).then_some(Point {
                day: r.day,
                value: w / hr,
                context: w,
                from_logbook: r.from_logbook,
            })
        })
        .collect();

    let mut weeks: Vec<Week> = Vec::new();
    if let Some(first) = rows.first() {
        let mut start = week_start(first.day);
        while start <= today {
            let inside = rows.iter().filter(|r| week_start(r.day) == start);
            let (n, m) = inside.fold((0, 0.0), |(n, m), r| (n + 1, m + r.minutes));
            weeks.push(Week { start_day: start, rows: n, minutes: m });
            start += 7;
        }
    }
    let mut streak_weeks = 0;
    let mut iter = weeks.iter().rev().peekable();
    // This week counts if it's already made it; otherwise start from last week.
    if iter.peek().is_some_and(|w| w.rows < WEEK_TARGET) {
        iter.next();
    }
    for w in iter {
        if w.rows < WEEK_TARGET {
            break;
        }
        streak_weeks += 1;
    }

    let faults = rows
        .iter()
        .flat_map(|r| {
            r.faults.iter().filter(|f| f.of > 0 && f.fault.needs_camera()).map(|f| FaultPoint {
                day: r.day,
                fault: f.fault,
                rate: f.hits as f64 / f.of as f64,
            })
        })
        .collect();

    let benchmarks = rows
        .iter()
        .filter_map(|r| {
            if r.plan.as_deref() == Some("rate20") {
                return Some(Benchmark { day: r.day, kind: "rate20".into(), value: r.watts_at_20.or(r.watts)? });
            }
            (r.distance_m.is_some_and(|d| (d - 2000.0).abs() < 1.0) && !r.intervals).then(|| Benchmark {
                day: r.day,
                kind: "2k".into(),
                value: r.minutes * 60.0,
            })
        })
        .collect();

    let recent = &halves[halves.len().saturating_sub(FATIGUE_SESSIONS)..];
    let fatigue = Fault::ALL
        .into_iter()
        .filter(|f| f.needs_camera())
        .filter_map(|f| {
            let pairs: Vec<(f64, f64)> = recent.iter().filter_map(|h| h.iter().find(|x| x.0 == f).map(|x| (x.1, x.2))).collect();
            (!pairs.is_empty()).then(|| Fatigue {
                fault: f,
                first_half: pairs.iter().map(|p| p.0).sum::<f64>() / pairs.len() as f64,
                second_half: pairs.iter().map(|p| p.1).sum::<f64>() / pairs.len() as f64,
                sessions: pairs.len() as u32,
            })
        })
        .collect();

    Progress {
        today,
        load,
        watts_at_20,
        ef,
        ef_watts: EF_WATTS,
        weeks,
        streak_weeks,
        faults,
        benchmarks,
        fatigue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: i64 = 20_395; // a Monday (2025-11-03)

    fn row(day: i64, minutes: f64, watts: f64, spm: f64, hr: f64) -> ProgressRow {
        ProgressRow {
            day,
            minutes,
            watts: Some(watts),
            spm: Some(spm),
            hr: Some(hr),
            from_logbook: true,
            ..Default::default()
        }
    }

    #[test]
    fn weeks_start_on_monday() {
        assert_eq!(week_start(0), -3, "1970-01-01 was a Thursday");
        assert_eq!(week_start(TODAY), TODAY);
        assert_eq!(week_start(TODAY + 6), TODAY);
        assert_eq!(week_start(TODAY - 1), TODAY - 7);
    }

    #[test]
    fn load_builds_and_decays() {
        let p = HeartProfile::new(60.0, 190.0, None).unwrap();
        // Three rows a week for four weeks, then a week off.
        let rows: Vec<ProgressRow> = (0..4)
            .flat_map(|w| [0, 2, 4].map(move |d| row(TODAY - 35 + w * 7 + d, 30.0, 53.0, 20.0, 140.0)))
            .collect();
        let pr = progress(&rows, &[], Some(&p), TODAY);
        let last = pr.load.last().unwrap();
        let peak = pr.load.iter().map(|d| d.fitness).fold(0.0, f64::max);
        assert!(last.fitness > 0.0 && last.fitness < peak, "fitness fades in the week off");
        assert!(last.form > 0.0, "fresh after a week off: form {}", last.form);
        assert_eq!(pr.streak_weeks, 0, "this week and last are empty");
        assert_eq!(pr.weeks.iter().filter(|w| w.rows == 3).count(), 4);
        // Without a profile, logbook rows without heart data add no load.
        assert!(progress(&rows, &[], None, TODAY).load.iter().all(|d| d.trimp == 0.0));
    }

    #[test]
    fn ef_only_from_comparable_steady_rows() {
        let rows = [
            row(TODAY - 10, 30.0, 52.0, 19.0, 140.0), // counts
            row(TODAY - 9, 17.0, 83.0, 20.0, 155.0),  // too hard to compare
            row(TODAY - 8, 10.0, 50.0, 20.0, 140.0),  // too short
            ProgressRow { intervals: true, ..row(TODAY - 7, 30.0, 52.0, 19.0, 140.0) },
        ];
        let pr = progress(&rows, &[], None, TODAY);
        assert_eq!(pr.ef.len(), 1);
        assert!((pr.ef[0].value - 52.0 / 140.0).abs() < 1e-9);
        // The 83 W row at rate 20 is a watts-at-20 point; the 10-minute one too.
        let w20: Vec<f64> = pr.watts_at_20.iter().map(|p| p.value).collect();
        assert_eq!(w20, vec![83.0, 50.0]);
    }

    #[test]
    fn streak_counts_full_weeks_back_from_now() {
        let mut rows = Vec::new();
        for w in 1..=3 {
            for d in [0, 2, 4] {
                rows.push(row(TODAY - 7 * w + d, 25.0, 50.0, 19.0, 135.0));
            }
        }
        rows.push(row(TODAY, 25.0, 50.0, 19.0, 135.0)); // this week: 1 so far
        assert_eq!(progress(&rows, &[], None, TODAY).streak_weeks, 3);
    }

    #[test]
    fn benchmarks_and_fatigue() {
        let mut test = row(TODAY - 3, 20.0, 88.0, 20.0, 160.0);
        test.plan = Some("rate20".into());
        test.watts_at_20 = Some(87.0);
        let mut two_k = row(TODAY - 1, 8.5, 150.0, 28.0, 175.0);
        two_k.distance_m = Some(2000.0);
        let pr = progress(&[test, two_k], &[vec![(Fault::EarlyArms, 0.1, 0.4)], vec![(Fault::EarlyArms, 0.0, 0.2)]], None, TODAY);
        assert_eq!(pr.benchmarks.len(), 2);
        assert_eq!((pr.benchmarks[0].kind.as_str(), pr.benchmarks[0].value), ("rate20", 87.0));
        assert_eq!((pr.benchmarks[1].kind.as_str(), pr.benchmarks[1].value), ("2k", 510.0));
        let f = pr.fatigue.iter().find(|f| f.fault == Fault::EarlyArms).unwrap();
        assert!((f.first_half - 0.05).abs() < 1e-9 && (f.second_half - 0.3).abs() < 1e-9);
    }
}
