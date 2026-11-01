//! `record` and `watch`: run the devices, show what they send, and pace the row.

use std::path::PathBuf;
use std::time::Duration;

use analysis::cues::{FocusCoach, Pacer};
use analysis::strokes::{StrokeAssembler, StrokeRecord};
use analysis::technique::{Fault, LiveTechnique};
use analysis::zones::HeartProfile;
use anyhow::Result;
use clap::{Args, ValueEnum};
use devices::{h10, pm5, sim};
use model::{Clock, Device, Event, EventKind, Pm5Status};
use recording::{FileWriter, Header};
use tokio::sync::mpsc;

use crate::ctx::Ctx;
use crate::plan::{Plan, describe_targets, parse_rate, parse_watts};
use crate::render::{clock, record_line};
use crate::review;
use crate::simulate::FaultArg;
use crate::speak::Speaker;

/// The drag factor is checked once it has settled, after this many strokes.
const DRAG_CHECK_STROKE: u32 = 5;
const DRAG_TOLERANCE: i16 = 5;
const STATUS_EVERY: Duration = Duration::from_secs(5);

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
    /// Serve the camera page and record its pose landmarks with the session
    #[arg(long)]
    camera: bool,
    /// Address for the camera page (with --camera)
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: std::net::SocketAddr,
    /// The one technique fault to coach this session (auto: the most
    /// frequent one in your last 3 sessions)
    #[arg(long, value_enum, default_value_t = FocusArg::Auto)]
    focus: FocusArg,
    /// With --sim: also simulate the camera (a stick-figure rower)
    #[arg(long, requires = "sim")]
    sim_camera: bool,
    /// With --sim: the simulated rower makes this fault (implies --sim-camera)
    #[arg(long, value_enum, requires = "sim")]
    sim_fault: Option<FaultArg>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FocusArg {
    Auto,
    None,
    ShortCompression,
    ShootingSlide,
    EarlyArms,
    RushedRecovery,
}

impl LiveArgs {
    /// Will strokes have a camera view?
    fn camera_on(&self) -> bool {
        self.camera || self.sim_camera || self.sim_fault.is_some()
    }
}

/// Faults rarer than this in recent sessions aren't worth a session's focus.
const FOCUS_MIN_RATE: f64 = 0.10;
const FOCUS_SESSIONS: usize = 3;

async fn choose_focus(args: &LiveArgs, ctx: Option<&Ctx>) -> Option<Fault> {
    let chosen = match args.focus {
        FocusArg::None => return None,
        FocusArg::ShortCompression => Fault::ShortCompression,
        FocusArg::ShootingSlide => Fault::ShootingSlide,
        FocusArg::EarlyArms => Fault::EarlyArms,
        FocusArg::RushedRecovery => Fault::RushedRecovery,
        FocusArg::Auto => {
            let sessions = store::sessions::list_sessions(&ctx?.pool).await.ok()?;
            let recent: Vec<_> = sessions.iter().rev().filter(|s| !s.meta.simulated).take(FOCUS_SESSIONS).collect();
            let rate = |f: Fault| {
                let rates: Vec<f64> = recent
                    .iter()
                    .filter_map(|s| s.summary.faults.iter().find(|r| r.fault == f && r.of > 0))
                    .map(|r| r.hits as f64 / r.of as f64)
                    .collect();
                (!rates.is_empty()).then(|| rates.iter().sum::<f64>() / rates.len() as f64)
            };
            let (fault, r) = Fault::ALL
                .into_iter()
                .filter(|f| args.camera_on() || !f.needs_camera())
                .filter_map(|f| rate(f).map(|r| (f, r)))
                .max_by(|a, b| a.1.total_cmp(&b.1))?;
            if r < FOCUS_MIN_RATE {
                return None;
            }
            println!("Focus picked from your last sessions: {} on {:.0}% of strokes", fault.label(), r * 100.0);
            fault
        }
    };
    if chosen.needs_camera() && !args.camera_on() {
        eprintln!("Focus {} needs the camera (--camera); not coaching it", chosen.label());
        return None;
    }
    Some(chosen)
}

/// Everything the live loop needs besides the devices.
struct Coach {
    heart: Option<HeartProfile>,
    drag_target: Option<u8>,
    pacer: Option<Pacer>,
    technique: LiveTechnique,
    focus: Option<FocusCoach>,
    speaker: Option<Speaker>,
}

impl Coach {
    fn new(args: &LiveArgs, ctx: Option<&Ctx>, focus: Option<Fault>) -> Self {
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
        if let Some(f) = focus {
            println!("Focus: {} (\"{}\")", f.label(), f.cue());
        }
        Self {
            heart,
            drag_target: ctx.and_then(|c| c.profile.drag_target),
            pacer,
            technique: LiveTechnique::new(),
            focus: focus.map(FocusCoach::new),
            speaker,
        }
    }

    fn cue(&self, text: &str) {
        println!("  » {text}");
        if let Some(s) = &self.speaker {
            s.say(text);
        }
    }

    /// Give a completed stroke its camera view and faults.
    fn observe(&mut self, r: &mut StrokeRecord) {
        self.technique.stroke(r);
    }

    /// React to a completed (observed) stroke.
    fn react(&mut self, r: &StrokeRecord, count: u32) {
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
        // Technique first: one cue per stroke at most.
        let technique = self.focus.as_mut().and_then(|f| f.stroke(r));
        let pacing = self.pacer.as_mut().and_then(|p| p.stroke(r));
        match (technique, pacing) {
            (Some(t), _) => self.cue(t.text()),
            (None, Some(p)) => self.cue(&p.text()),
            (None, None) => {}
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
    let focus = choose_focus(&args, ctx.as_ref()).await;
    let coach = Coach::new(&args, ctx.as_ref(), focus);
    println!("Recording to {} (Ctrl-C to stop)", path.display());

    let pool = ctx.as_ref().map(|c| c.pool.clone());
    let (writer, strokes) = run(&args, clock, Some(writer), coach, pool).await?;
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
    let focus = choose_focus(&args, ctx.as_ref()).await;
    let coach = Coach::new(&args, ctx.as_ref(), focus);
    let pool = ctx.as_ref().map(|c| c.pool.clone());
    run(&args, Clock::start(), None, coach, pool).await.map(|_| ())
}

/// Returns the writer (unfinished) and the number of completed strokes.
async fn run(
    args: &LiveArgs,
    clock: Clock,
    mut writer: Option<FileWriter>,
    mut coach: Coach,
    pool: Option<store::SqlitePool>,
) -> Result<(Option<FileWriter>, u32)> {
    let (tx, mut rx) = mpsc::channel::<Event>(4096);
    let mut tasks = Vec::new();
    if args.sim {
        let cfg = sim::SimConfig {
            pose: (args.sim_camera || args.sim_fault.is_some()).then(|| devices::sim_pose::PoseSim {
                fault: args.sim_fault.map(Into::into),
                ..Default::default()
            }),
            ..Default::default()
        };
        tasks.push(tokio::spawn(sim::run(clock, tx.clone(), cfg)));
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
    // The camera page's frames join the same stream as the devices'.
    let camera = if args.camera {
        let app = server::router(pool, &crate::default_web_dir(), server::Live::into_session(clock, tx.clone()));
        match server::listen(args.listen, app).await {
            Ok((addr, task)) => {
                println!("Camera page: http://{addr}/camera (open it on this machine: browsers only allow the camera on localhost)");
                Some(task)
            }
            Err(e) => {
                eprintln!("Camera page unavailable ({e:#}); is `rowctl serve` already using that port?");
                None
            }
        }
    } else {
        None
    };
    drop(tx);

    let start_us = clock.now_us();
    let mut view = View {
        heart: coach.heart,
        ..Default::default()
    };
    let mut asm = StrokeAssembler::new();
    let mut ticker = tokio::time::interval(STATUS_EVERY);
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
                if let EventKind::Pose(p) = &ev.kind {
                    coach.technique.frame(p);
                }
                if let Some(mut r) = asm.push(&ev) {
                    view.strokes += 1;
                    coach.observe(&mut r);
                    println!("{}", record_line(&r, start_us, view.heart.as_ref()));
                    coach.react(&r, view.strokes);
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
    if let Some(task) = camera {
        task.abort();
    }
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
    camera_link: Option<bool>,
    /// Pose frames since the last status line.
    poses: u32,
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
            EventKind::Pose(_) => self.poses += 1,
            EventKind::Link(link) => {
                let (slot, name) = match link.device {
                    Device::Pm5 => (&mut self.pm5_link, "PM5"),
                    Device::H10 => (&mut self.h10_link, "H10"),
                    Device::Camera => (&mut self.camera_link, "Camera page"),
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

    /// Call every `STATUS_EVERY`.
    fn status_line(&mut self, elapsed_us: i64) -> String {
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
        let cam = match self.camera_link {
            Some(true) => format!(" | cam {:.0} fps", self.poses as f64 / STATUS_EVERY.as_secs_f64()),
            _ => String::new(),
        };
        self.poses = 0;
        format!(
            "[{}] {pm5} | {hr}{cam} | {} strokes, {} events",
            clock(elapsed_us as f64 / 1e6),
            self.strokes,
            self.events
        )
    }
}

fn show<T: ToString>(v: Option<T>) -> String {
    v.map(|v| v.to_string()).unwrap_or_else(|| "–".into())
}

