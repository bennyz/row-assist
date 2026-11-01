//! `record` and `watch`: run the devices, show what they send, and pace the row.

use std::path::PathBuf;
use std::time::Duration;

use analysis::cues::Pacer;
use analysis::strokes::{StrokeAssembler, StrokeRecord};
use analysis::zones::HeartProfile;
use anyhow::Result;
use clap::Args;
use devices::{h10, pm5, sim};
use model::{Clock, Device, Event, EventKind, Pm5Status};
use recording::{FileWriter, Header};
use tokio::sync::mpsc;

use crate::ctx::Ctx;
use crate::plan::{Plan, describe_targets, parse_rate, parse_watts};
use crate::render::{clock, record_line};
use crate::review;
use crate::speak::Speaker;

/// The drag factor is checked once it has settled, after this many strokes.
const DRAG_CHECK_STROKE: u32 = 5;
const DRAG_TOLERANCE: i16 = 5;

#[derive(Args, Debug, Clone)]
pub struct LiveArgs {
    /// Use simulated devices instead of hardware
    #[arg(long)]
    sim: bool,
    /// Don't connect to the PM5
    #[arg(long)]
    no_pm5: bool,
    /// Don't connect to the heart-rate strap
    #[arg(long)]
    no_h10: bool,
    /// Connect to the heart-rate strap whose name contains this
    #[arg(long, default_value = "H10")]
    h10_name: String,
    /// Stop after this many seconds
    #[arg(long)]
    seconds: Option<u64>,
    /// Row a preset workout (sets the targets below)
    #[arg(long, value_enum)]
    plan: Option<Plan>,
    /// Stroke-rate range for cues, e.g. 19-21 (one number means ±1)
    #[arg(long, value_parser = parse_rate)]
    rate: Option<(f64, f64)>,
    /// Power range for cues, e.g. 48-58 (one number means ±5)
    #[arg(long, value_parser = parse_watts)]
    watts: Option<(f64, f64)>,
    /// Cue when heart rate goes above the top of this zone (1-5)
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
    zone: Option<u8>,
    /// Piece length, for the halfway and time calls
    #[arg(long)]
    minutes: Option<f64>,
    /// Speak cues aloud (macOS `say`; Linux `spd-say` or `espeak-ng`)
    #[arg(long)]
    speak: bool,
    /// Database [default: <data>/row-assist.db]
    #[arg(long)]
    db: Option<PathBuf>,
}

/// Everything the live loop needs besides the devices.
struct Coach {
    heart: Option<HeartProfile>,
    drag_target: Option<u8>,
    pacer: Option<Pacer>,
    speaker: Option<Speaker>,
}

impl Coach {
    fn new(args: &LiveArgs, ctx: Option<&Ctx>) -> Self {
        let heart = ctx.and_then(|c| c.heart()).map(|h| h.0);
        let mut targets = args.plan.map(|p| p.targets(heart.as_ref())).unwrap_or_default();
        if let Some(p) = args.plan {
            println!("{}", p.describe());
        }
        targets.rate = args.rate.or(targets.rate);
        targets.watts = args.watts.or(targets.watts);
        targets.minutes = args.minutes.or(targets.minutes);
        if let Some(z) = args.zone {
            match heart.as_ref().and_then(|h| h.zone_bpm(z)) {
                Some((_, top)) => targets.hr_max = Some(top),
                None => eprintln!("--zone needs a heart profile (see rowctl profile); ignoring it"),
            }
        }
        if let Some(c) = ctx {
            println!("Zones from {}", c.heart_note());
        }

        let speaker = if args.speak {
            let s = Speaker::start();
            match &s {
                Some(s) => println!("Speaking cues with {}", s.program),
                None => eprintln!(
                    "--speak: no speech program found (looked for {}); cues will only be printed",
                    Speaker::candidates().join(", ")
                ),
            }
            s
        } else {
            None
        };
        let pacer = (!targets.is_empty()).then(|| {
            println!("Cues: {}", describe_targets(&targets));
            Pacer::new(targets)
        });
        Self {
            heart,
            drag_target: ctx.and_then(|c| c.profile.drag_target),
            pacer,
            speaker,
        }
    }

    fn cue(&self, text: &str) {
        println!("  » {text}");
        if let Some(s) = &self.speaker {
            s.say(text);
        }
    }

    /// Called for every completed stroke.
    fn stroke(&mut self, r: &StrokeRecord, count: u32) {
        if count == DRAG_CHECK_STROKE
            && let Some(df) = r.drag_factor
        {
            match self.drag_target {
                Some(t) if (df as i16 - t as i16).abs() > DRAG_TOLERANCE => {
                    self.cue(&format!("Drag factor {df}. Target {t}."));
                }
                Some(_) => {}
                None => println!("  Drag factor {df} (no target: set one with rowctl profile --drag {df})"),
            }
        }
        if let Some(cue) = self.pacer.as_mut().and_then(|p| p.stroke(r)) {
            self.cue(&cue.text());
        }
    }
}

/// The database is useful for zones and saving, but recording must not
/// depend on it.
async fn open_ctx(args: &LiveArgs) -> Option<Ctx> {
    match Ctx::open(args.db.clone()).await {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("Database unavailable ({e:#}): no zones, and the session won't be saved");
            None
        }
    }
}

pub async fn record(args: LiveArgs, out: Option<PathBuf>, note: Option<String>) -> Result<()> {
    let ctx = open_ctx(&args).await;
    let clock = Clock::start();
    let path = out.unwrap_or_else(|| {
        let name = chrono::Local::now().format("%Y%m%d-%H%M%S");
        store::paths::sessions_dir().join(format!("{name}.rowrec"))
    });
    let mut header = Header::new(clock.now_us(), concat!("rowctl ", env!("CARGO_PKG_VERSION")));
    header.note = note;
    header.simulated = args.sim;
    header.plan = args.plan.map(|p| p.name().to_owned());
    let writer = FileWriter::create(&path, &header)?;
    let coach = Coach::new(&args, ctx.as_ref());
    println!("Recording to {} (Ctrl-C to stop)", path.display());

    let (writer, strokes) = run(&args, clock, Some(writer), coach).await?;
    let Some(w) = writer else { return Ok(()) };
    let events = w.events();
    w.finish()?;
    println!("\nSaved {events} events, {strokes} strokes to {}\n", path.display());

    match &ctx {
        Some(ctx) => match review::process(ctx, &path, true).await {
            Ok(done) => done.print(ctx).await?,
            Err(e) => eprintln!("Analysis failed ({e:#}); retry with: rowctl analyze {}", path.display()),
        },
        None => println!("Analyse and save later with: rowctl analyze {}", path.display()),
    }
    Ok(())
}

pub async fn watch(args: LiveArgs) -> Result<()> {
    let ctx = open_ctx(&args).await;
    let coach = Coach::new(&args, ctx.as_ref());
    run(&args, Clock::start(), None, coach).await.map(|_| ())
}

/// Returns the writer (unfinished) and the number of completed strokes.
async fn run(
    args: &LiveArgs,
    clock: Clock,
    mut writer: Option<FileWriter>,
    mut coach: Coach,
) -> Result<(Option<FileWriter>, u32)> {
    let (tx, mut rx) = mpsc::channel::<Event>(4096);
    let mut tasks = Vec::new();
    if args.sim {
        tasks.push(tokio::spawn(sim::run(clock, tx.clone(), sim::SimConfig::default())));
    } else {
        if !args.no_pm5 {
            let tx = tx.clone();
            // hidapi is blocking: give the PM5 its own thread. It exits when
            // the channel closes.
            std::thread::spawn(move || pm5::run_blocking(clock, tx, pm5::Pm5Config::default()));
        }
        if !args.no_h10 {
            let cfg = h10::H10Config {
                name_contains: args.h10_name.clone(),
                ..Default::default()
            };
            tasks.push(tokio::spawn(h10::run(clock, tx.clone(), cfg)));
        }
    }
    drop(tx);

    let start_us = clock.now_us();
    let mut view = View {
        heart: coach.heart,
        ..Default::default()
    };
    let mut asm = StrokeAssembler::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(5));
    ticker.tick().await;
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    let seconds = args.seconds;
    let deadline = async move {
        match seconds {
            Some(s) => tokio::time::sleep(Duration::from_secs(s)).await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            ev = rx.recv() => {
                let Some(ev) = ev else { break };
                if let Some(w) = writer.as_mut() {
                    w.write(&ev)?;
                }
                if let Some(msg) = view.apply(&ev) {
                    println!("{msg}");
                }
                if let Some(r) = asm.push(&ev) {
                    view.strokes += 1;
                    println!("{}", record_line(&r, start_us, view.heart.as_ref()));
                    coach.stroke(&r, view.strokes);
                }
            }
            _ = ticker.tick() => println!("{}", view.status_line(clock.now_us() - start_us)),
            _ = &mut ctrl_c => break,
            _ = &mut deadline => break,
        }
    }

    // Closing the channel stops the device tasks; give the H10 a moment to
    // disconnect cleanly so the strap is free for the PM5 or phone.
    drop(rx);
    for task in tasks {
        let _ = tokio::time::timeout(Duration::from_secs(3), task).await;
    }
    Ok((writer, view.strokes))
}

#[derive(Default)]
struct View {
    heart: Option<HeartProfile>,
    pm5: Option<Pm5Status>,
    pm5_link: Option<bool>,
    h10_link: Option<bool>,
    bpm: Option<u16>,
    events: u64,
    strokes: u32,
}

impl View {
    /// Track the latest values. Returns a message when a device's link changes.
    fn apply(&mut self, ev: &Event) -> Option<String> {
        self.events += 1;
        match &ev.kind {
            EventKind::Pm5Status(s) => self.pm5 = Some(s.clone()),
            EventKind::Heart(h) => self.bpm = Some(h.bpm),
            EventKind::Link(link) => {
                let (slot, name) = match link.device {
                    Device::Pm5 => (&mut self.pm5_link, "PM5"),
                    Device::H10 => (&mut self.h10_link, "H10"),
                    Device::Camera => return None,
                };
                // Announce changes, not every failed reconnect attempt.
                if *slot == Some(link.connected) {
                    return None;
                }
                *slot = Some(link.connected);
                let detail = link.detail.as_deref().map(|d| format!(": {d}")).unwrap_or_default();
                let what = if link.connected { "connected" } else { "unavailable" };
                return Some(format!("{name} {what}{detail}"));
            }
            _ => {}
        }
        None
    }

    fn status_line(&self, elapsed_us: i64) -> String {
        let pm5 = match (self.pm5_link, &self.pm5) {
            (Some(true), Some(s)) => format!(
                "PM5 {} {} spm {} W {} m DF {}",
                s.stroke_state.map(|st| st.label()).unwrap_or("?"),
                show(s.spm),
                show(s.watts),
                show(s.distance_dm.map(|d| d / 10)),
                show(s.drag_factor),
            ),
            (Some(true), None) => "PM5 connected, no data yet".into(),
            _ => "PM5 –".into(),
        };
        let hr = match (self.h10_link, self.bpm) {
            (Some(true), Some(b)) => match &self.heart {
                Some(h) => format!("HR {b} Z{}", h.zone(b as f64)),
                None => format!("HR {b}"),
            },
            (Some(true), None) => "HR …".into(),
            _ => "HR –".into(),
        };
        format!(
            "[{}] {pm5} | {hr} | {} strokes, {} events",
            clock(elapsed_us as f64 / 1e6),
            self.strokes,
            self.events
        )
    }
}

fn show<T: ToString>(v: Option<T>) -> String {
    v.map(|v| v.to_string()).unwrap_or_else(|| "–".into())
}

