//! `report`: how Row Assist itself did on each recorded row, for whoever
//! works on it next. Per row: what the camera delivered, how the strokes
//! were judged, what the coach said and whether the fault it named became
//! rarer afterwards, what dropped, and what the rower wrote on the review
//! screen. Everything is worked out again from the recording, so a row from
//! months ago is judged by today's analysis.

use std::path::PathBuf;

use analysis::ideal::{Span, Spans};
use analysis::strokes::StrokeRecord;
use analysis::technique::{Fault, fault_rates, target_sequence};
use anyhow::{Result, bail};
use model::{Device, Event, EventKind};
use store::sessions::Feel;

use crate::ctx::{Ctx, local};
use crate::render::clock;
use crate::review::all_recordings;

pub async fn run(files: Vec<PathBuf>, db: Option<PathBuf>) -> Result<()> {
    let ctx = Ctx::open(db).await?;
    let files = if files.is_empty() { all_recordings()? } else { files };
    if files.is_empty() {
        bail!("no recordings in {}", store::paths::sessions_dir().display());
    }
    let sessions = store::sessions::list_sessions(&ctx.pool).await?;
    for file in &files {
        let (header, events, truncated) = recording::read_all(file)?;
        let saved = file.canonicalize()?.display().to_string();
        let feel = sessions.iter().find(|s| s.meta.recording == saved).map(|s| &s.feel);
        let when = local(header.started_at_us)
            .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        let tags = [
            header.simulated.then_some("simulated"),
            truncated.then_some("cut off mid-stream"),
        ];
        let tags: Vec<&str> = tags.into_iter().flatten().collect();
        let tags = if tags.is_empty() {
            String::new()
        } else {
            format!("  ({})", tags.join(", "))
        };
        println!("{when}  {}{tags}", file.display());
        for line in lines(&events, feel) {
            println!("  {line}");
        }
        println!();
    }
    Ok(())
}

/// The report on one recording, a line per subject, each starting with its
/// name. A subject with nothing to say (no camera, no cues) has no line.
fn lines(events: &[Event], feel: Option<&Feel>) -> Vec<String> {
    let a = analysis::analyze(events, None);
    let mut out = Vec::new();
    match &a.summary {
        Some(s) => out.push(format!(
            "row      {}, {} strokes, {} W, {} spm",
            clock(s.whole.seconds),
            s.whole.strokes,
            s.whole.watts.map_or("–".into(), |w| format!("{w:.0}")),
            s.whole.spm.map_or("–".into(), |r| format!("{r:.1}")),
        )),
        None => out.push("row      no strokes".into()),
    }
    out.extend(camera(events, &a.strokes));
    let rates: Vec<String> = fault_rates(&a.strokes)
        .iter()
        .map(|r| format!("{} {}/{}", r.fault.label(), r.hits, r.of))
        .collect();
    if !rates.is_empty() {
        out.push(format!("faults   {}", rates.join(", ")));
    }
    out.extend(order(&a.strokes));
    out.extend(cues(events, &a.strokes));
    let drops: Vec<String> = [Device::Pm5, Device::H10, Device::Camera]
        .into_iter()
        .filter_map(|d| {
            let n = events
                .iter()
                .filter(|e| matches!(&e.kind, EventKind::Link(l) if l.device == d && !l.connected))
                .count();
            (n > 0).then(|| format!("{d:?} ×{n}"))
        })
        .collect();
    if !drops.is_empty() {
        out.push(format!("dropped  {}", drops.join(", ")));
    }
    if let Some(f) = feel
        && (f.effort.is_some() || f.comment.is_some())
    {
        let effort = f.effort.map(|e| format!("effort {e}")).into_iter();
        let comment = f.comment.as_ref().map(|c| format!("{c:?}")).into_iter();
        out.push(format!(
            "rower    {}",
            effort.chain(comment).collect::<Vec<_>>().join(", ")
        ));
    }
    out
}

/// The value a `share` of the sorted values lie under.
fn quantile(sorted: &[f64], share: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * share).round() as usize]
}

/// What the camera page delivered: how many frames, how evenly, how long
/// each took from capture to reaching the app (pose detection on the phone
/// and the network; the page keeps its clock in step with the app's), and
/// how many of the strokes the analysis could read from them.
fn camera(events: &[Event], strokes: &[StrokeRecord]) -> Option<String> {
    let frames: Vec<(i64, i64)> = events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Pose(p) => Some((p.captured_us, e.t_us)),
            _ => None,
        })
        .collect();
    let (first, last) = (frames.first()?, frames.last()?);
    let seconds = (last.0 - first.0) as f64 / 1e6;
    let mut gaps: Vec<f64> = frames.windows(2).map(|w| (w[1].0 - w[0].0) as f64 / 1e3).collect();
    let mut delays: Vec<f64> = frames
        .iter()
        .map(|&(captured, arrived)| (arrived - captured) as f64 / 1e3)
        .collect();
    gaps.sort_by(f64::total_cmp);
    delays.sort_by(f64::total_cmp);
    let complete = strokes.iter().filter(|s| s.t_next_catch_us.is_some());
    let (read, of) = complete.fold((0, 0), |(read, of), s| (read + usize::from(s.pose.is_some()), of + 1));
    Some(format!(
        "camera   {} frames, {:.0}/s, longest gap {:.0} ms; reached the app {:.0} ms after capture (19 in 20 within {:.0}); read {read} of {of} strokes",
        frames.len(),
        (frames.len() - 1) as f64 / seconds,
        gaps.last().copied().unwrap_or(0.0),
        quantile(&delays, 0.5),
        quantile(&delays, 0.95),
    ))
}

/// Where in the handle's travel each part did its moving, as percentages:
/// the median stroke beside the good stroke for this rower's build.
fn order(strokes: &[StrokeRecord]) -> Vec<String> {
    let Some(good) = target_sequence(strokes) else {
        return Vec::new();
    };
    let median = |pick: fn(&StrokeRecord) -> Option<Span>| -> Option<Span> {
        let mut starts: Vec<f64> = Vec::new();
        let mut ends: Vec<f64> = Vec::new();
        for span in strokes.iter().filter_map(pick) {
            starts.push(span.start);
            ends.push(span.end);
        }
        starts.sort_by(f64::total_cmp);
        ends.sort_by(f64::total_cmp);
        (!starts.is_empty()).then(|| Span {
            start: quantile(&starts, 0.5),
            end: quantile(&ends, 0.5),
        })
    };
    let drive = Spans {
        legs: median(|s| s.pose.as_ref()?.sequence.drive.legs),
        body: median(|s| s.pose.as_ref()?.sequence.drive.body),
        arms: median(|s| s.pose.as_ref()?.sequence.drive.arms),
    };
    let recovery = Spans {
        legs: median(|s| s.pose.as_ref()?.sequence.recovery.legs),
        body: median(|s| s.pose.as_ref()?.sequence.recovery.body),
        arms: median(|s| s.pose.as_ref()?.sequence.recovery.arms),
    };
    let span = |s: Option<Span>| s.map_or("–".into(), |s| format!("{:.0}–{:.0}", s.start * 100.0, s.end * 100.0));
    let row = |d: &Spans, r: &Spans| {
        format!(
            "drive legs {} body {} arms {} | recovery arms {} body {} legs {}",
            span(d.legs),
            span(d.body),
            span(d.arms),
            span(r.arms),
            span(r.body),
            span(r.legs)
        )
    };
    vec![
        format!("order    you   {}", row(&drive, &recovery)),
        format!("         good  {}", row(&good.drive, &good.recovery)),
    ]
}

/// Each thing the coach said, how often and from when. For a technique cue,
/// also how often its fault was flagged before the coach first said it and
/// after: whether saying it changed the rowing.
fn cues(events: &[Event], strokes: &[StrokeRecord]) -> Vec<String> {
    let start = strokes.first().map_or(0, |s| s.t_catch_us);
    let mut said: Vec<(&str, i64, usize)> = Vec::new();
    for e in events {
        let EventKind::Cue { text } = &e.kind else {
            continue;
        };
        match said.iter_mut().find(|(t, _, _)| t == text) {
            Some(cue) => cue.2 += 1,
            None => said.push((text, e.t_us, 1)),
        }
    }
    said.into_iter()
        .enumerate()
        .map(|(i, (text, first_us, times))| {
            let name = if i == 0 { "cues" } else { "" };
            let at = clock((first_us - start).max(0) as f64 / 1e6);
            let effect = Fault::ALL
                .into_iter()
                .find(|f| f.cue() == text)
                .map(|fault| {
                    let (before, after): (Vec<&StrokeRecord>, Vec<&StrokeRecord>) = strokes
                        .iter()
                        .filter(|s| s.t_next_catch_us.is_some() && (!fault.needs_camera() || s.pose.is_some()))
                        .partition(|s| s.t_catch_us < first_us);
                    let hits =
                        |of: &[&StrokeRecord]| of.iter().filter(|s| s.faults.iter().any(|h| h.fault == fault)).count();
                    format!(
                        ": {} on {} of {} strokes before it, {} of {} after",
                        fault.label(),
                        hits(&before),
                        before.len(),
                        hits(&after),
                        after.len()
                    )
                })
                .unwrap_or_default();
            format!("{name:<8} {text:?} ×{times} from {at}{effect}")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use devices::sim::{SimConfig, generate};
    use devices::sim_pose::{PoseSim, SimFault};

    use super::*;

    const START_US: i64 = 1_794_000_000_000_000;

    fn row(fault: Option<SimFault>) -> Vec<Event> {
        let cfg = SimConfig {
            pose: Some(PoseSim {
                fault,
                ..Default::default()
            }),
            ..Default::default()
        };
        generate(cfg, START_US, Duration::from_secs(90))
    }

    fn line<'a>(lines: &'a [String], name: &str) -> &'a str {
        lines
            .iter()
            .find(|l| l.starts_with(name))
            .map(String::as_str)
            .unwrap_or_else(|| panic!("no {name} line in {lines:#?}"))
    }

    #[test]
    fn a_row_with_a_fault_reports_the_camera_the_fault_the_cue_and_the_rower() {
        let mut events = row(Some(SimFault::EarlyBody));
        // The coach names the fault a minute in, and a strap drops.
        let said = START_US + 60_000_000;
        let at = events.iter().position(|e| e.t_us >= said).unwrap();
        events.insert(
            at,
            Event {
                t_us: said,
                kind: EventKind::Cue {
                    text: Fault::EarlyBody.cue().into(),
                },
            },
        );
        events.push(Event {
            t_us: START_US + 89_000_000,
            kind: EventKind::Link(model::Link {
                device: Device::H10,
                connected: false,
                detail: None,
            }),
        });
        let feel = Feel {
            effort: Some(6),
            comment: Some("target looked too upright".into()),
        };
        let lines = lines(&events, Some(&feel));

        let camera = line(&lines, "camera");
        assert!(camera.contains("frames") && !camera.contains("read 0 of"), "{camera}");
        let faults = line(&lines, "faults");
        let early_body = faults.split(", ").find(|f| f.contains("early body")).unwrap();
        let (hits, of) = early_body.rsplit_once(' ').unwrap().1.split_once('/').unwrap();
        assert!(
            hits.parse::<f64>().unwrap() > 0.8 * of.parse::<f64>().unwrap(),
            "{faults}"
        );
        assert!(line(&lines, "order").contains("you"));
        let cue = line(&lines, "cues");
        // A minute of a 90 s row at 20 spm: 20 strokes before the cue, the rest after.
        assert_eq!(
            cue,
            "cues     \"Legs, then swing\" ×1 from 01:00: early body on 20 of 20 strokes before it, 9 of 9 after"
        );
        assert_eq!(line(&lines, "dropped"), "dropped  H10 ×1");
        assert_eq!(
            line(&lines, "rower"),
            "rower    effort 6, \"target looked too upright\""
        );
    }

    #[test]
    fn a_clean_row_without_a_camera_reports_only_what_there_was() {
        let events = generate(SimConfig::default(), START_US, Duration::from_secs(60));
        let lines = lines(&events, None);
        assert!(line(&lines, "row").contains("strokes"));
        for absent in ["camera", "order", "cues", "dropped", "rower"] {
            assert!(!lines.iter().any(|l| l.starts_with(absent)), "{absent} in {lines:#?}");
        }
    }
}
