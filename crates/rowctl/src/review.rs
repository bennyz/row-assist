//! `analyze` and `sessions`: turn recordings into session summaries, store
//! them, and print them.

use std::path::{Path, PathBuf};

use analysis::Analysis;
use analysis::metrics::{format_pace, pace_from_watts};
use analysis::reference::{self, Outcome};
use analysis::session::{Segment, Summary};
use anyhow::{Result, bail};
use recording::Header;
use store::sessions::SessionMeta;

use crate::ctx::{Ctx, local};
use crate::plan::num;
use crate::render::clock;

pub async fn analyze(files: Vec<PathBuf>, all: bool, no_save: bool, db: Option<PathBuf>) -> Result<()> {
    let ctx = Ctx::open(db).await?;
    let files = if all { all_recordings()? } else { files };
    if files.is_empty() {
        bail!("no recordings: pass files, or --all for everything in {}", store::paths::sessions_dir().display());
    }
    let brief = files.len() > 1;
    for file in &files {
        match process(&ctx, file, !no_save).await {
            Ok(done) if brief => println!("{}", done.brief(file)),
            Ok(done) => done.print(&ctx).await?,
            Err(e) if brief => println!("{}: {e:#}", file.display()),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn all_recordings() -> Result<Vec<PathBuf>> {
    let dir = store::paths::sessions_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "rowrec"))
        .collect();
    files.sort();
    Ok(files)
}

/// A recording, analysed (and saved, if asked).
pub struct Processed {
    pub header: Header,
    pub analysis: Analysis,
    pub truncated: bool,
    pub saved: Option<i64>,
}

pub async fn process(ctx: &Ctx, file: &Path, save: bool) -> Result<Processed> {
    let (header, events, truncated) = recording::read_all(file)?;
    let heart = ctx.heart().map(|h| h.0);
    let analysis = analysis::analyze(&events, heart.as_ref());
    let saved = match (&analysis.summary, save) {
        (Some(summary), true) => {
            let meta = SessionMeta {
                started_at_us: header.started_at_us,
                recording: file.canonicalize().unwrap_or(file.to_path_buf()).display().to_string(),
                note: header.note.clone(),
                plan: header.plan.clone(),
                simulated: header.simulated,
            };
            Some(store::sessions::save_session(&ctx.pool, &meta, summary, &analysis.strokes).await?)
        }
        _ => None,
    };
    Ok(Processed {
        header,
        analysis,
        truncated,
        saved,
    })
}

impl Processed {
    fn brief(&self, file: &Path) -> String {
        let Some(s) = &self.analysis.summary else {
            return format!("{}: no strokes", file.display());
        };
        format!(
            "{}: {} strokes, {} min, {} W, HR {}{}",
            file.display(),
            s.whole.strokes,
            num1(s.whole.seconds / 60.0),
            opt(s.whole.watts, 0),
            opt(s.whole.hr, 0),
            if self.saved.is_some() { ", saved" } else { "" }
        )
    }

    pub async fn print(&self, ctx: &Ctx) -> Result<()> {
        let h = &self.header;
        let when = local(h.started_at_us)
            .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        let mut tags = Vec::new();
        if let Some(p) = &h.plan {
            tags.push(format!("plan {p}"));
        }
        if let Some(n) = &h.note {
            tags.push(format!("{n:?}"));
        }
        if h.simulated {
            tags.push("simulated".into());
        }
        let tags = if tags.is_empty() { String::new() } else { format!("  ({})", tags.join(", ")) };
        println!("Session {when}{tags}");

        let Some(s) = &self.analysis.summary else {
            println!("  No strokes: was the PM5 connected and the flywheel spinning?");
            return Ok(());
        };
        print_summary(s, ctx);

        if s.whole.seconds >= reference::MIN_MINUTES * 60.0 || h.plan.as_deref() == Some("reference") {
            let history = ctx.history(s.started_at_us).await?;
            print_reference(&s.whole, &history, h.plan.as_deref() == Some("reference"));
        }
        if let Some(peak) = s.heart.as_ref().and_then(|x| x.peak_10s_bpm)
            && let Some((hp, _, _)) = ctx.heart()
            && peak > hp.max + 0.5
            && !h.simulated
        {
            println!(
                "  Heart rate held {peak:.0} for 10 s, above HRmax {:.0}. If that was real effort \
                 (not a dry strap), raise it: rowctl profile --hr-max {peak:.0}",
                hp.max
            );
        }
        if self.truncated {
            println!("  (recording ends mid-stream: the writer didn't finish)");
        }
        match self.saved {
            Some(id) => println!("  Saved as session {id} in {}", ctx.db_path.display()),
            None => println!("  Not saved"),
        }
        Ok(())
    }
}

fn print_summary(s: &Summary, ctx: &Ctx) {
    let w = &s.whole;
    let pace = w.watts.map(|x| format!(", {} /500m", format_pace(pace_from_watts(x)))).unwrap_or_default();
    let partial = if s.partial_strokes > 0 {
        format!(" (+{} partial)", s.partial_strokes)
    } else {
        String::new()
    };
    println!(
        "  {} rowed, {} strokes{partial}, {} m{pace}",
        clock(w.seconds),
        w.strokes,
        opt(w.distance_m, 0)
    );
    let jps = w.watts.zip(w.spm).map(|(watts, spm)| watts * 60.0 / spm);
    let mps = w.distance_m.map(|d| d / w.strokes.max(1) as f64);
    println!(
        "  {} W  {} spm  {} J/stroke  {} m/stroke  drive {} s  ratio {}  peak {} N",
        opt(w.watts, 0),
        opt(w.spm, 1),
        opt(jps, 0),
        opt(mps, 2),
        opt(s.drive_ms.map(|d| d / 1000.0), 2),
        opt(s.ratio, 1),
        opt(s.peak_n, 0),
    );
    if let Some(hs) = &s.heart {
        println!(
            "  HR {:.0} avg, {} peak (10 s), {} max   EF {}   decoupling {}",
            hs.mean_bpm,
            opt(hs.peak_10s_bpm, 0),
            hs.max_bpm,
            opt(w.ef, 3),
            s.decoupling_pct.map(|d| format!("{d:.1}%")).unwrap_or_else(|| "– (needs 13+ min)".into()),
        );
        if !hs.zone_s.is_empty() {
            let zones: Vec<String> = hs
                .zone_s
                .iter()
                .enumerate()
                .filter(|&(_, &secs)| secs >= 1.0)
                .map(|(z, &secs)| format!("Z{z} {}", clock(secs)))
                .collect();
            println!("  {}   TRIMP {}", zones.join("  "), opt(hs.trimp, 0));
            println!("  ({})", ctx.heart_note());
        }
    } else {
        println!("  No heart rate");
    }
    if let Some(seg) = &s.rate_20 {
        println!("  Watts at rate 20: {} W over {} strokes", opt(seg.watts, 0), seg.strokes);
    }
    if let Some(seg) = &s.last_10_min {
        println!("  Last 10 min: {} W, HR {}, EF {}", opt(seg.watts, 0), opt(seg.hr, 0), opt(seg.ef, 3));
    }
    if let Some(cam) = &s.camera {
        println!(
            "  Camera: {} side, tilt {:.1}°, {}",
            cam.side.label(),
            // No "-0.0".
            if cam.tilt_deg.abs() < 0.05 { 0.0 } else { cam.tilt_deg },
            cam.offset_ms
                .map(|o| format!("PM5 catch {o:.0} ms after the camera's"))
                .unwrap_or_else(|| "not enough strokes to sync".into())
        );
    }
    let faults: Vec<String> = s
        .faults
        .iter()
        .filter(|r| r.of > 0)
        .map(|r| format!("{} {}%", r.fault.label(), (r.hits as f64 * 100.0 / r.of as f64).round()))
        .collect();
    if !faults.is_empty() {
        println!("  Faults (share of strokes): {}", faults.join(", "));
    }
    if let Some(df) = s.drag_factor {
        let target = match ctx.profile.drag_target {
            Some(t) if (df as i16 - t as i16).abs() > 5 => format!(" (target {t}: more than 5 off)"),
            Some(t) => format!(" (target {t})"),
            None => " (no target set: rowctl profile --drag N)".into(),
        };
        println!("  Drag factor {df}{target}");
    }
}

fn print_reference(today: &Segment, history: &[reference::Piece], is_reference: bool) {
    match reference::compare(today, history) {
        Outcome::Compared(c) => {
            println!(
                "  EF {:.3} vs {} past pieces at {:.0}–{:.0} W: best {:.3}, worst {:.3}",
                c.ef,
                c.pieces.len(),
                c.watts - reference::WATTS_WINDOW,
                c.watts + reference::WATTS_WINDOW,
                c.high,
                c.low
            );
            if is_reference {
                println!("  Reference row → {}", c.ramp.describe());
            }
        }
        Outcome::NoBaseline { ef, watts, pieces } => println!(
            "  EF {ef:.3}; only {pieces} past steady pieces near {watts:.0} W, need {} to compare",
            reference::MIN_PIECES
        ),
        Outcome::NotSteady(why) if is_reference => println!("  Not usable as a reference row: {why}"),
        Outcome::NotSteady(_) => {}
    }
}

pub async fn sessions(db: Option<PathBuf>) -> Result<()> {
    let ctx = Ctx::open(db).await?;
    let rows = store::sessions::list_sessions(&ctx.pool).await?;
    if rows.is_empty() {
        println!("No sessions yet. Record one with: rowctl record");
        return Ok(());
    }
    println!(
        "{:>4} {:<16} {:>5} {:>5} {:>4} {:>4} {:>4} {:>5} {:>5} {:>4} {:>3} {:>5}  notes",
        "id", "started", "min", "m", "spm", "W", "HR", "EF", "dec%", "W@20", "DF", "TRIMP"
    );
    for r in &rows {
        let s = &r.summary;
        let w = &s.whole;
        let mut notes: Vec<String> = Vec::new();
        notes.extend(r.meta.plan.clone());
        notes.extend(r.meta.note.as_ref().map(|n| format!("{n:?}")));
        if r.meta.simulated {
            notes.push("sim".into());
        }
        println!(
            "{:>4} {:<16} {:>5} {:>5} {:>4} {:>4} {:>4} {:>5} {:>5} {:>4} {:>3} {:>5}  {}",
            r.id,
            local(s.started_at_us).map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default(),
            num1(w.seconds / 60.0),
            opt(w.distance_m, 0),
            opt(w.spm, 1),
            opt(w.watts, 0),
            opt(w.hr, 0),
            opt(w.ef, 3),
            opt(s.decoupling_pct, 1),
            opt(s.rate_20.as_ref().and_then(|x| x.watts), 0),
            s.drag_factor.map(|d| d.to_string()).unwrap_or_else(|| "–".into()),
            opt(s.heart.as_ref().and_then(|h| h.trimp), 0),
            notes.join(", "),
        );
    }
    Ok(())
}

fn opt(v: Option<f64>, digits: usize) -> String {
    v.map(|v| format!("{v:.digits$}")).unwrap_or_else(|| "–".into())
}

fn num1(v: f64) -> String {
    num((v * 10.0).round() / 10.0)
}
