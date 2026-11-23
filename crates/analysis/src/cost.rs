//! What the drive faults go with in work: each stroke's work, from the PM5,
//! held against how the camera saw its drive.
//!
//! A fault shown on nearly every stroke leaves no
//! clean strokes to compare with, so strokes are compared by how far they
//! showed it: the worst third against the best third by the measure behind
//! the fault, whether or not it crossed the fault's line. Power drifts through
//! a row (intervals, rate changes, tiring) and a measure can drift with it, so
//! each stroke's work is first taken against its neighbours'. What comes out
//! is what goes together, not proof of cause.

use serde::{Deserialize, Serialize};

use crate::ideal::median;
use crate::session::MAX_CYCLE_S;
use crate::strokes::{CURVE_POINTS, StrokeRecord};
use crate::technique::{Fault, StrokePose};

/// Strokes each side whose work a stroke's is held against.
const NEIGHBOURS: usize = 5;
/// With fewer neighbours than this (a short run) a stroke isn't compared.
const MIN_NEIGHBOURS: usize = 4;
/// After a piece or an interval starts: strokes still bringing the flywheel up to speed.
const SETTLE_US: i64 = 10_000_000;
/// Strokes each third needs before the thirds are compared.
const MIN_THIRD: usize = 15;
/// How far apart the thirds' ranks must sit (Mann–Whitney z) to be more
/// than stroke-to-stroke noise.
const CLEAR_Z: f64 = 2.0;

/// One drive fault against work per stroke.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaultCost {
    pub fault: Fault,
    /// Strokes in each third.
    pub strokes: u32,
    /// The measure behind the fault, the median of the worst and of the best
    /// third: degrees for the shin from vertical at the catch, the trunk's tip
    /// forward and the elbow; for early body, the share of the drive the
    /// handle had come when the body started.
    pub worst: f64,
    pub best: f64,
    /// Work per stroke of the worst third against the best, percent, each
    /// stroke's against its neighbours': negative, the worst third did less.
    pub work_pct: f64,
    /// Median power of the strokes compared, W: what the percentage is of.
    pub watts: f64,
    /// The thirds differ by more than stroke-to-stroke noise.
    pub clear: bool,
    /// Mean drive force of each third, newtons, catch to finish: where in
    /// the drive they part. Each stroke's curve is scaled by the typical
    /// stroke's neighbours' work over its own neighbours', as its work is, so
    /// drift through the row doesn't show.
    pub worst_curve: Vec<f64>,
    pub best_curve: Vec<f64>,
}

/// Each drive fault the camera measured on enough strokes, the clear ones
/// first, costliest first. `windows`: the work intervals
/// (`workout::work_windows`); with none, the row is one piece.
pub fn fault_costs(strokes: &[StrokeRecord], windows: &[(i64, i64)]) -> Vec<FaultCost> {
    let compared: Vec<(&StrokeRecord, Work)> = runs(strokes, windows)
        .iter()
        .flat_map(|run| run.iter().copied().zip(relative_work(run)))
        .filter_map(|(s, work)| Some((s, work?)))
        .collect();
    let mut costs: Vec<FaultCost> = Fault::ALL.iter().filter_map(|&f| cost(f, &compared)).collect();
    costs.sort_by(|a, b| b.clear.cmp(&a.clear).then(a.work_pct.total_cmp(&b.work_pct)));
    costs
}

/// Strokes whose power belongs together: each work interval, or the whole
/// row. Complete strokes only, once the flywheel is up to speed.
fn runs<'a>(strokes: &'a [StrokeRecord], windows: &[(i64, i64)]) -> Vec<Vec<&'a StrokeRecord>> {
    let Some(first) = strokes.first() else {
        return Vec::new();
    };
    let whole = [(first.t_catch_us, i64::MAX)];
    let spans = if windows.is_empty() { &whole[..] } else { windows };
    spans
        .iter()
        .map(|&(from, to)| {
            strokes
                .iter()
                .filter(|s| {
                    s.t_catch_us >= from.saturating_add(SETTLE_US)
                        && s.t_catch_us < to
                        && s.cycle_s().is_some_and(|c| c > 0.0 && c <= MAX_CYCLE_S)
                })
                .collect()
        })
        .collect()
}

/// A stroke's work against its neighbours'.
#[derive(Debug, Clone, Copy)]
struct Work {
    /// Its own over `near`.
    ratio: f64,
    /// The median of its neighbours' in the run, J.
    near: f64,
}

/// Each stroke's work against its neighbours' in the run.
fn relative_work(run: &[&StrokeRecord]) -> Vec<Option<Work>> {
    let joules: Vec<Option<f64>> = run.iter().map(|s| s.joules()).collect();
    (0..run.len())
        .map(|i| {
            let own = joules[i]?;
            let near: Vec<f64> = (i.saturating_sub(NEIGHBOURS)..(i + NEIGHBOURS + 1).min(run.len()))
                .filter(|&j| j != i)
                .filter_map(|j| joules[j])
                .collect();
            if near.len() < MIN_NEIGHBOURS {
                return None;
            }
            median(near).filter(|&m| m > 0.0).map(|m| Work { ratio: own / m, near: m })
        })
        .collect()
}

/// The measure behind a drive fault on one stroke, and 1 when larger is
/// worse (-1 when smaller is). The recovery's faults have none: they would
/// show in the next drive, if anywhere.
fn measure(fault: Fault, p: &StrokePose) -> Option<(f64, f64)> {
    match fault {
        Fault::ShortCompression => p.catch.shin.map(|v| (v, 1.0)),
        Fault::ShootingSlide => p.early_trunk_gain.map(|v| (v, 1.0)),
        Fault::EarlyBody => p.sequence.drive.body.map(|b| (b.start, -1.0)),
        Fault::EarlyArms => p.elbow_while_legs_bent.map(|v| (v, -1.0)),
        Fault::RushedRecovery | Fault::EarlyKnees => None,
    }
}

/// A compared stroke as one fault sees it.
struct Seen<'a> {
    /// Larger is worse.
    badness: f64,
    value: f64,
    stroke: &'a StrokeRecord,
    work: Work,
}

fn cost(fault: Fault, compared: &[(&StrokeRecord, Work)]) -> Option<FaultCost> {
    let mut seen: Vec<Seen> = compared
        .iter()
        .filter_map(|&(stroke, work)| {
            let (value, worse) = measure(fault, stroke.pose.as_ref()?)?;
            Some(Seen {
                badness: value * worse,
                value,
                stroke,
                work,
            })
        })
        .collect();
    let third = seen.len() / 3;
    if third < MIN_THIRD {
        return None;
    }
    seen.sort_by(|a, b| b.badness.total_cmp(&a.badness));
    let (worst, best) = (&seen[..third], &seen[seen.len() - third..]);
    let (worst_work, best_work) = (works(worst), works(best));
    let typical = median(seen.iter().map(|x| x.work.near).collect())?;
    Some(FaultCost {
        fault,
        strokes: third as u32,
        worst: median(worst.iter().map(|x| x.value).collect())?,
        best: median(best.iter().map(|x| x.value).collect())?,
        work_pct: (median(worst_work.clone())? / median(best_work.clone())? - 1.0) * 100.0,
        watts: median(seen.iter().filter_map(|x| x.stroke.watts).map(f64::from).collect())?,
        clear: rank_z(&worst_work, &best_work).abs() >= CLEAR_Z,
        worst_curve: mean_curve(worst, typical),
        best_curve: mean_curve(best, typical),
    })
}

fn works(seen: &[Seen]) -> Vec<f64> {
    seen.iter().map(|x| x.work.ratio).collect()
}

/// Point-by-point mean of the strokes' drive force, each scaled to a stroke
/// whose neighbours did `typical` work (empty when none has a curve).
fn mean_curve(seen: &[Seen], typical: f64) -> Vec<f64> {
    let curves: Vec<(&Vec<f64>, f64)> = seen
        .iter()
        .filter(|x| x.stroke.force_n.len() == CURVE_POINTS)
        .map(|x| (&x.stroke.force_n, typical / x.work.near))
        .collect();
    if curves.is_empty() {
        return Vec::new();
    }
    (0..CURVE_POINTS)
        .map(|i| curves.iter().map(|(c, scale)| c[i] * scale).sum::<f64>() / curves.len() as f64)
        .collect()
}

/// Mann–Whitney's z for `a` against `b`: how far apart their ranks sit, in
/// standard errors (positive: `a` larger), whatever the shape of either.
fn rank_z(a: &[f64], b: &[f64]) -> f64 {
    let u: f64 = a
        .iter()
        .map(|x| {
            b.iter()
                .map(|y| match x.total_cmp(y) {
                    std::cmp::Ordering::Greater => 1.0,
                    std::cmp::Ordering::Equal => 0.5,
                    std::cmp::Ordering::Less => 0.0,
                })
                .sum::<f64>()
        })
        .sum();
    let (n, m) = (a.len() as f64, b.len() as f64);
    (u - n * m / 2.0) / (n * m * (n + m + 1.0) / 12.0).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CYCLE_US: i64 = 3_000_000;

    /// Stroke `i` with this much work and these arms; its curve is as high
    /// as its work.
    fn stroke(i: usize, joules: f64, elbow: f64) -> StrokeRecord {
        let mut s = StrokeRecord::empty();
        s.index = i as u32;
        s.t_catch_us = i as i64 * CYCLE_US;
        s.t_next_catch_us = Some(s.t_catch_us + CYCLE_US);
        s.watts = Some((joules / 3.0).round() as u16);
        s.force_n = vec![joules; CURVE_POINTS];
        s.pose = Some(StrokePose {
            elbow_while_legs_bent: Some(elbow),
            ..StrokePose::default()
        });
        s
    }

    /// Stroke-to-stroke noise that has nothing to do with the arms.
    fn wobble(i: usize) -> f64 {
        ((i * 5) % 7) as f64 * 3.0
    }

    #[test]
    fn arms_bent_early_go_with_less_work() {
        // Every third stroke bends the arms early and does 10% less.
        let strokes: Vec<_> = (0..120)
            .map(|i| {
                let early = i % 3 == 0;
                let work = if early { 270.0 } else { 300.0 };
                stroke(i, work + wobble(i), if early { 120.0 } else { 150.0 })
            })
            .collect();
        let costs = fault_costs(&strokes, &[]);
        assert_eq!(costs.len(), 1, "only the arms were measured");
        let c = &costs[0];
        assert_eq!(c.fault, Fault::EarlyArms);
        assert!(c.clear, "{c:?}");
        assert!((-12.0..-8.0).contains(&c.work_pct), "{}", c.work_pct);
        assert_eq!((c.worst, c.best), (120.0, 150.0));
        assert!(c.worst_curve[0] < c.best_curve[0]);
    }

    #[test]
    fn a_row_that_fades_doesnt_blame_the_arms() {
        // Work fades through the row and the arms bend earlier as it goes,
        // both from tiring: stroke by stroke, the arms make no difference.
        let strokes: Vec<_> = (0..120)
            .map(|i| {
                let t = i as f64 / 119.0;
                stroke(i, 330.0 - 90.0 * t + wobble(i), 160.0 - 40.0 * t)
            })
            .collect();
        let c = &fault_costs(&strokes, &[])[0];
        assert!(!c.clear, "{c:?}");
        assert!(c.work_pct.abs() < 2.0, "{}", c.work_pct);
        // Nor do the force curves: the late strokes' are as high as their neighbours.
        let gap = c.worst_curve[0] / c.best_curve[0] - 1.0;
        assert!(gap.abs() < 0.03, "the curves part by {:.1}%", gap * 100.0);
    }

    #[test]
    fn only_work_strokes_are_compared() {
        // Rests rowed light with the arms bent stay out, as do each
        // interval's first strokes.
        let strokes: Vec<_> = (0..150)
            .map(|i| {
                let rest = (i / 30) % 2 == 1;
                let (work, elbow) = if rest { (100.0, 110.0) } else { (300.0, 140.0 + wobble(i)) };
                stroke(i, work + wobble(i), elbow)
            })
            .collect();
        let windows: Vec<(i64, i64)> = [0, 60, 120]
            .iter()
            .map(|&k| (k * CYCLE_US, (k + 30) * CYCLE_US))
            .collect();
        let c = &fault_costs(&strokes, &windows)[0];
        // Three intervals of 30 strokes, less the 4 in each one's first 10 s.
        assert_eq!(c.strokes, 26);
        assert!(c.worst >= 140.0, "no rest stroke compared: {c:?}");
    }

    #[test]
    fn too_few_strokes_say_nothing() {
        let strokes: Vec<_> = (0..40).map(|i| stroke(i, 300.0 + wobble(i), 150.0)).collect();
        assert!(fault_costs(&strokes, &[]).is_empty());
    }
}
