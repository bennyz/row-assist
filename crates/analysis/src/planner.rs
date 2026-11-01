//! What to row today, and why (DESIGN.md §8). Explicit rules over the rows
//! done so far and today's readiness; every proposal carries its reasons.

use serde::{Deserialize, Serialize};

use crate::readiness::Level;
use crate::reference::Ramp;
use crate::technique::Fault;

/// A row done, from the logbook or the app. `day` counts days since 1970-01-01.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub day: i64,
    pub minutes: f64,
    /// The plan it was rowed to (`reference`, `easy`, `steady`, `rate20`…), if known.
    pub plan: Option<String>,
    /// For a reference row: what it said about the comeback ramp.
    pub ramp: Option<Ramp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// 20 min at rate 20, ~53 W: measures what fitness is left after a gap.
    Reference,
    /// 20–25 min easy, rate 18–20, zone 2 or below.
    Easy,
    /// 30–45 min zone 2, rate 18–20.
    Steady,
    /// Rate steps at matched form, e.g. 4 × 5 min at 20 / 22 / 24 / 22.
    RateSteps,
    /// 20 min held at rate 20, best steady power: the benchmark.
    Rate20Test,
    /// 15–20 min easy: better than skipping.
    Minimum,
}

impl Kind {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Reference => "reference row: 20 min at rate 20, about 53 W",
            Self::Easy => "easy row: rate 18–20, heart rate in zone 2 or below",
            Self::Steady => "steady row: zone 2, rate 18–20",
            Self::RateSteps => "rate steps: 4 × 5 min at rate 20 / 22 / 24 / 22, 2 min easy between, same form at every rate",
            Self::Rate20Test => "rate-20 test: 20 min held at exactly 20 spm, best steady power",
            Self::Minimum => "minimum session: easy, rate 18–20",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Phase {
    /// No reference row yet in this block.
    Restart,
    /// Easy weeks after the reference row.
    Comeback { week: u32, of: u32 },
    /// A gap of 8–21 days: one easy week before intensity.
    EasyWeek,
    /// The normal weekly template.
    Build { week: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub kind: Kind,
    pub minutes: (u32, u32),
    pub phase: Phase,
    pub focus: Option<Fault>,
    /// Rows in the last 7 days, and the aim.
    pub rows_this_week: u32,
    pub target_per_week: u32,
    pub why: Vec<String>,
}

/// Rows shorter than this are warm-ups or false starts.
pub const MIN_ROW_MINUTES: f64 = 10.0;
/// A gap longer than this starts over with a reference row.
pub const RESTART_GAP_DAYS: i64 = 21;
/// A gap longer than this gets an easy week first.
pub const EASY_WEEK_GAP_DAYS: i64 = 7;
/// Past this many days without a row, say so.
pub const NUDGE_DAYS: i64 = 4;
pub const ROWS_PER_WEEK: u32 = 3;
/// Build weeks before the first rate-20 test.
pub const TEST_AFTER_WEEKS: u32 = 3;

fn weeks_for(ramp: Option<Ramp>) -> u32 {
    match ramp {
        Some(Ramp::OneWeek) => 1,
        Some(Ramp::TwoWeeks) | None => 2,
        Some(Ramp::Full) => 3,
    }
}

/// `rows` in any order; `focus` is the technique point chosen elsewhere.
pub fn propose(rows: &[Row], today: i64, readiness: Level, focus: Option<Fault>) -> Proposal {
    let mut rows: Vec<&Row> = rows.iter().filter(|r| r.minutes >= MIN_ROW_MINUTES && r.day <= today).collect();
    rows.sort_by_key(|r| r.day);
    let mut why = Vec::new();

    // The current block starts after the last long gap (or today, after one).
    let last_day = rows.last().map(|r| r.day);
    let gap = last_day.map(|d| today - d);
    let mut block_start = rows.first().map_or(today, |r| r.day);
    for w in rows.windows(2) {
        if w[1].day - w[0].day > RESTART_GAP_DAYS {
            block_start = w[1].day;
        }
    }
    if gap.is_none_or(|g| g > RESTART_GAP_DAYS) {
        block_start = today;
    }
    let block: Vec<&Row> = rows.iter().copied().filter(|r| r.day >= block_start).collect();
    let rows_this_week = rows.iter().filter(|r| r.day > today - 7).count() as u32;

    match gap {
        None => why.push("no rows yet".into()),
        Some(g) if g > RESTART_GAP_DAYS => why.push(format!("{g} days since the last row: start over")),
        Some(g) if g >= NUDGE_DAYS => why.push(format!("{g} days since the last row; keep gaps under a week")),
        _ => {}
    }

    let reference = block.iter().rev().find(|r| r.plan.as_deref() == Some("reference"));
    // Gap inside the block before today, for the easy week.
    let recent_gap = rows.windows(2).last().map(|w| w[1].day - w[0].day).filter(|_| gap == Some(0));
    let restarted_after = gap.filter(|&g| g > EASY_WEEK_GAP_DAYS && g <= RESTART_GAP_DAYS).or(recent_gap.filter(|&g| g > EASY_WEEK_GAP_DAYS && g <= RESTART_GAP_DAYS));

    let (mut kind, mut minutes, phase) = if let Some(r) = reference {
        let of = weeks_for(r.ramp);
        let since = today - r.day;
        let week = (since / 7) as u32 + 1;
        if week <= of {
            why.push(format!(
                "comeback ramp week {week} of {of} (from the reference row{})",
                r.ramp.map(|x| format!(": {}", x.describe())).unwrap_or_default()
            ));
            (Kind::Easy, (20, 25), Phase::Comeback { week, of })
        } else {
            let build_week = ((since - 7 * of as i64) / 7) as u32 + 1;
            build(&block, today, build_week, &mut why)
        }
    } else {
        why.push("first row of this block: measure where fitness is".into());
        (Kind::Reference, (20, 20), Phase::Restart)
    };

    let mut phase = phase;
    if let (Some(g), Phase::Build { .. }) = (restarted_after, phase) {
        why.push(format!("back after {g} days: an easy week before intensity"));
        phase = Phase::EasyWeek;
        if matches!(kind, Kind::RateSteps | Kind::Rate20Test) {
            kind = Kind::Easy;
            minutes = (20, 30);
        }
    }

    match readiness {
        Level::Red if kind != Kind::Reference => {
            why.push("readiness red: minimum session".into());
            kind = Kind::Minimum;
            minutes = (15, 20);
        }
        Level::Red => why.push("readiness red: the reference row can wait a day if you feel it".into()),
        Level::Amber if matches!(kind, Kind::RateSteps | Kind::Rate20Test) => {
            why.push("readiness amber: no intensity today".into());
            kind = Kind::Steady;
            minutes = (30, 35);
        }
        _ => {}
    }
    if rows_this_week >= ROWS_PER_WEEK && kind != Kind::Reference {
        why.push(format!("{rows_this_week} rows in the last 7 days already: this one is optional"));
    }

    Proposal {
        kind,
        minutes,
        phase,
        focus,
        rows_this_week,
        target_per_week: ROWS_PER_WEEK,
        why,
    }
}

/// The weekly template: 2 steady, 1 rate work, benchmark when due, a lighter
/// fourth week.
fn build(block: &[&Row], today: i64, week: u32, why: &mut Vec<String>) -> (Kind, (u32, u32), Phase) {
    let phase = Phase::Build { week };
    let this_week: Vec<&&Row> = block.iter().filter(|r| r.day > today - 7).collect();
    let did = |plan: &str| this_week.iter().any(|r| r.plan.as_deref() == Some(plan));
    let tested = block.iter().any(|r| r.plan.as_deref() == Some("rate20"));
    let lighter = week.is_multiple_of(4);
    let steady = if lighter { (25, 30) } else { (30, 45) };

    if week >= TEST_AFTER_WEEKS && !tested {
        why.push(format!("{week} weeks of steady rowing: time for the rate-20 benchmark"));
        return (Kind::Rate20Test, (20, 20), phase);
    }
    if lighter {
        why.push(format!("build week {week}: a lighter week"));
    } else {
        why.push(format!("build week {week}"));
    }
    let steadies = this_week.iter().filter(|r| matches!(r.plan.as_deref(), Some("steady" | "easy") | None)).count();
    if !lighter && !did("rate_steps") && steadies >= 1 {
        why.push("one rate session a week, once the steady rows are in".into());
        return (Kind::RateSteps, (30, 35), phase);
    }
    (Kind::Steady, steady, phase)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: i64 = 20_400;

    fn row(day: i64, plan: Option<&str>) -> Row {
        Row {
            day,
            minutes: 25.0,
            plan: plan.map(Into::into),
            ramp: None,
        }
    }

    fn reference(day: i64, ramp: Ramp) -> Row {
        Row {
            ramp: Some(ramp),
            ..row(day, Some("reference"))
        }
    }

    #[test]
    fn after_a_long_gap_start_with_a_reference_row() {
        let rows = [row(TODAY - 200, None), row(TODAY - 197, None)];
        let p = propose(&rows, TODAY, Level::Unknown, None);
        assert_eq!((p.kind, p.phase), (Kind::Reference, Phase::Restart));
        assert!(p.why[0].starts_with("197 days since the last row"));
        assert_eq!(propose(&[], TODAY, Level::Green, None).kind, Kind::Reference);
    }

    #[test]
    fn the_reference_verdict_sets_the_ramp_length() {
        let rows = [reference(TODAY - 9, Ramp::TwoWeeks), row(TODAY - 7, Some("easy")), row(TODAY - 4, Some("easy"))];
        let p = propose(&rows, TODAY, Level::Green, Some(Fault::EarlyArms));
        assert_eq!(p.kind, Kind::Easy);
        assert_eq!(p.phase, Phase::Comeback { week: 2, of: 2 });
        assert_eq!(p.focus, Some(Fault::EarlyArms));
        // Fitness held: one week, so on day 9 the build has started.
        let rows = [reference(TODAY - 9, Ramp::OneWeek), row(TODAY - 4, Some("easy"))];
        assert!(matches!(propose(&rows, TODAY, Level::Green, None).phase, Phase::Build { week: 1 }));
    }

    #[test]
    fn build_weeks_follow_the_template() {
        // One-week ramp from day -10, so today is build week 1 (days 7–13
        // after the reference row); a steady row this week → rate steps.
        let rows = [reference(TODAY - 10, Ramp::OneWeek), row(TODAY - 9, Some("easy")), row(TODAY - 2, Some("steady"))];
        let p = propose(&rows, TODAY, Level::Green, None);
        assert_eq!((p.kind, p.phase), (Kind::RateSteps, Phase::Build { week: 1 }));
        // Rate steps done this week → steady.
        let mut done = rows.to_vec();
        done.push(row(TODAY - 1, Some("rate_steps")));
        assert_eq!(propose(&done, TODAY, Level::Green, None).kind, Kind::Steady);
    }

    #[test]
    fn the_benchmark_comes_after_three_build_weeks_once() {
        let mut rows = vec![reference(TODAY - 28, Ramp::OneWeek)];
        // Every other day since, right up to yesterday (no gap to recover from).
        rows.extend((1..=27).step_by(2).map(|k| row(TODAY - 28 + k, Some("steady"))));
        let p = propose(&rows, TODAY, Level::Green, None);
        assert_eq!((p.kind, p.phase), (Kind::Rate20Test, Phase::Build { week: 4 }));
        rows.push(row(TODAY - 1, Some("rate20")));
        assert_ne!(propose(&rows, TODAY, Level::Green, None).kind, Kind::Rate20Test);
    }

    #[test]
    fn readiness_softens_the_day() {
        let rows = [reference(TODAY - 14, Ramp::OneWeek), row(TODAY - 2, Some("steady"))];
        assert_eq!(propose(&rows, TODAY, Level::Amber, None).kind, Kind::Steady, "no intensity on amber");
        let p = propose(&rows, TODAY, Level::Red, None);
        assert_eq!((p.kind, p.minutes), (Kind::Minimum, (15, 20)));
    }

    #[test]
    fn a_middling_gap_gets_an_easy_week() {
        let rows = [reference(TODAY - 40, Ramp::OneWeek), row(TODAY - 30, Some("steady")), row(TODAY - 26, Some("steady"))];
        let p = propose(&rows, TODAY - 12, Level::Green, None);
        assert_eq!(p.phase, Phase::EasyWeek);
        assert!(matches!(p.kind, Kind::Easy | Kind::Steady));
        assert!(p.why.iter().any(|w| w.contains("14 days since")));
    }

    #[test]
    fn says_when_the_week_is_already_full() {
        let rows = [reference(TODAY - 14, Ramp::OneWeek), row(TODAY - 5, Some("steady")), row(TODAY - 3, Some("rate_steps")), row(TODAY - 1, Some("steady"))];
        let p = propose(&rows, TODAY, Level::Green, None);
        assert_eq!(p.rows_this_week, 3);
        assert!(p.why.last().unwrap().contains("optional"));
    }
}
