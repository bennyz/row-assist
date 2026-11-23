//! `record` and `watch`: run the devices, show what they send, and pace the row.

use std::path::PathBuf;
use std::time::Duration;

use analysis::cues::{Cue, FocusCoach, Pacer, Targets};
use analysis::workout::{Kind, Progression, Workout};
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
use crate::plan::{Plan, describe_workout, parse_rate, parse_watts};
use crate::render::{clock, record_line};
use crate::review;
use crate::simulate::FaultArg;
use server::live::{self, LiveMsg, LiveSender, StrokeMsg};
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
    /// Speak cues aloud (`spd-say` or `espeak-ng`)
    #[arg(long)]
    speak: bool,
    /// Database [default: <data>/row-assist.db]
    #[arg(long)]
    db: Option<PathBuf>,
    /// Serve the camera page (its pose landmarks are recorded with the
    /// session) and the mirror for a front screen
    #[arg(long)]
    camera: bool,
    /// Address for the camera page (with --camera; --phone listens on all
    /// interfaces at this port)
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: std::net::SocketAddr,
    /// Use a phone on the network as the camera (implies --camera): the
    /// camera page is also served over HTTPS, and a QR code opens its
    /// one-time setup
    #[arg(long)]
    phone: bool,
    /// The one technique fault to coach this session (auto: the most
    /// frequent one in your last 3 sessions)
    #[arg(long, value_enum, default_value_t = FocusArg::Auto)]
    focus: FocusArg,
    /// Instead of devices: play a recording's events back at their own
    /// pace, as if they were happening now (with --camera, to watch a past
    /// row on the row screen)
    #[arg(long, conflicts_with = "sim")]
    replay: Option<PathBuf>,
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
    EarlyBody,
    EarlyArms,
    RushedRecovery,
    EarlyKnees,
}

impl FocusArg {
    /// The flag value that selects `fault` (checked against `Fault::flag`).
    #[cfg(test)]
    pub fn flag(fault: Fault) -> String {
        let arg = match fault {
            Fault::ShortCompression => Self::ShortCompression,
            Fault::ShootingSlide => Self::ShootingSlide,
            Fault::EarlyBody => Self::EarlyBody,
            Fault::EarlyArms => Self::EarlyArms,
            Fault::RushedRecovery => Self::RushedRecovery,
            Fault::EarlyKnees => Self::EarlyKnees,
        };
        arg.to_possible_value().map(|v| v.get_name().to_owned()).unwrap_or_default()
    }
}

impl LiveArgs {
    /// Will strokes have a camera view?
    fn camera_on(&self) -> bool {
        self.camera || self.phone || self.sim_camera || self.sim_fault.is_some() || self.replay.is_some()
    }

    fn setup(&self, focus: Option<Fault>) -> Setup {
        // Its power from last time comes from the database, in `record`.
        Setup {
            kind: self.plan.map(Plan::kind),
            progression: None,
            rate: self.rate,
            watts: self.watts,
            zone: self.zone,
            minutes: self.minutes,
            focus,
        }
    }

    fn devices(&self) -> Devices {
        Devices {
            sim: self.sim.then(|| sim::SimConfig {
                pose: (self.sim_camera || self.sim_fault.is_some()).then(|| devices::sim_pose::PoseSim {
                    fault: self.sim_fault.map(Into::into),
                    ..Default::default()
                }),
                intervals: self.plan.and_then(|p| p.kind().intervals()).map(sim_intervals),
                ..Default::default()
            }),
            pm5: !self.no_pm5,
            h10: (!self.no_h10).then(|| self.h10_name.clone()),
        }
    }
}

/// Which devices to run.
pub struct Devices {
    /// The simulator instead of hardware.
    pub sim: Option<sim::SimConfig>,
    pub pm5: bool,
    /// The heart-rate strap whose name contains this.
    pub h10: Option<String>,
}

/// Start the devices sending events on `tx`. They stop when its receiver is
/// dropped; the PM5 runs on its own thread (hidapi blocks).
pub fn spawn_devices(d: Devices, clock: Clock, tx: &mpsc::Sender<Event>) -> Vec<tokio::task::JoinHandle<()>> {
    let mut tasks = Vec::new();
    if let Some(cfg) = d.sim {
        tasks.push(tokio::spawn(sim::run(clock, tx.clone(), cfg)));
        return tasks;
    }
    if d.pm5 {
        let tx = tx.clone();
        std::thread::spawn(move || pm5::run_blocking(clock, tx, pm5::Pm5Config::default()));
    }
    if let Some(name) = d.h10 {
        let cfg = h10::H10Config {
            name_contains: name,
            ..Default::default()
        };
        tasks.push(tokio::spawn(h10::run(clock, tx.clone(), cfg)));
    }
    tasks
}

/// A recording's events again, at their own pace, moved to now. What the
/// coach said then is left out: this session's coach speaks for itself.
async fn replay(events: Vec<Event>, clock: Clock, tx: mpsc::Sender<Event>) {
    let Some(first) = events.first().map(|e| e.t_us) else { return };
    let shift = clock.now_us() - first;
    for mut ev in events {
        if matches!(ev.kind, EventKind::Cue { .. }) {
            continue;
        }
        ev.t_us += shift;
        if let EventKind::Pose(p) = &mut ev.kind {
            p.captured_us += shift;
        }
        let wait = ev.t_us - clock.now_us();
        if wait > 0 {
            tokio::time::sleep(Duration::from_micros(wait as u64)).await;
        }
        if tx.send(ev).await.is_err() {
            return;
        }
    }
}

/// Heart rate from one source: the strap's own connection when this machine
/// has it (with beat-to-beat intervals), else the PM5's relay of a strap
/// paired to it (bpm only). Both at once would count every beat twice.
#[derive(Debug, Default)]
pub struct HeartPick {
    direct: bool,
}

impl HeartPick {
    /// Whether to use `ev`.
    pub fn keep(&mut self, ev: &Event) -> bool {
        match &ev.kind {
            EventKind::Link(l) if l.device == Device::H10 => {
                self.direct = l.connected;
                true
            }
            EventKind::Heart(h) if h.via_pm5 => !self.direct,
            _ => true,
        }
    }
}

/// The simulated rower rows a workout's intervals as the PM5 would time them.
pub fn sim_intervals(i: analysis::workout::Intervals) -> sim::SimIntervals {
    sim::SimIntervals {
        count: i.count,
        work_s: i.work_s,
        rest_s: i.rest_s,
    }
}

/// What one row is set up as: a workout, its length where the plan (or a
/// flag) sets it, its power from last time, flags' targets over the
/// workout's, and the technique focus.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Setup {
    pub kind: Option<Kind>,
    pub progression: Option<Progression>,
    pub rate: Option<(f64, f64)>,
    pub watts: Option<(f64, f64)>,
    pub zone: Option<u8>,
    pub minutes: Option<f64>,
    pub focus: Option<Fault>,
}

impl Setup {
    /// The workout; `false` when `zone` needs a heart profile there isn't.
    pub fn workout(&self, heart: Option<&HeartProfile>) -> (Workout, bool) {
        let mut w = match self.kind {
            Some(k) => k.workout(heart, self.progression.as_ref()),
            None => Workout::Piece {
                minutes: None,
                targets: Targets::default(),
            },
        };
        if let Some(m) = self.minutes {
            w.set_minutes(m);
        }
        let zone_top = self.zone.map(|z| heart.and_then(|h| h.zone_bpm(z)).map(|(_, top)| top));
        for t in w.targets_mut() {
            t.rate = self.rate.or(t.rate);
            t.watts = self.watts.or(t.watts);
            if let Some(Some(top)) = zone_top {
                t.hr_max = Some(top);
            }
        }
        (w, zone_top != Some(None))
    }
}

/// Speech for cues, or `None` (and a note why) when there's no program.
pub fn start_speaker() -> Option<Speaker> {
    let s = Speaker::start();
    match &s {
        Some(s) => println!("Speaking cues with {}", s.program),
        None => eprintln!(
            "No speech program found (looked for {}); cues will only be shown",
            Speaker::candidates().join(", ")
        ),
    }
    s
}


async fn choose_focus(args: &LiveArgs, ctx: Option<&Ctx>) -> Option<Fault> {
    let chosen = match args.focus {
        FocusArg::None => return None,
        FocusArg::ShortCompression => Fault::ShortCompression,
        FocusArg::ShootingSlide => Fault::ShootingSlide,
        FocusArg::EarlyBody => Fault::EarlyBody,
        FocusArg::EarlyArms => Fault::EarlyArms,
        FocusArg::RushedRecovery => Fault::RushedRecovery,
        FocusArg::EarlyKnees => Fault::EarlyKnees,
        FocusArg::Auto => {
            let (fault, r) = ctx?.auto_focus(args.camera_on()).await?;
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

/// Everything the live loop needs besides the devices: one per row.
pub struct Coach {
    pub heart: Option<HeartProfile>,
    drag_target: Option<u8>,
    pacer: Option<Pacer>,
    technique: LiveTechnique,
    focus: Option<FocusCoach>,
    speaker: Option<Speaker>,
    /// Screens following the session (the mirror), when serving.
    pub screens: Option<LiveSender>,
    /// What the coach said, and the workout's stages, since the recorder
    /// last took them.
    said: Vec<EventKind>,
}

impl Coach {
    pub fn new(setup: &Setup, ctx: Option<&Ctx>, speaker: Option<Speaker>) -> Self {
        let heart = ctx.and_then(|c| c.heart()).map(|h| h.0);
        if let Some(k) = setup.kind {
            println!("{}", k.describe());
        }
        if let Some(p) = &setup.progression {
            println!("{}", p.note);
        }
        let (workout, zone_ok) = setup.workout(heart.as_ref());
        if !zone_ok {
            eprintln!("--zone needs a heart profile (see rowctl profile); ignoring it");
        }
        if let Some(c) = ctx {
            println!("Zones from {}", c.heart_note());
        }
        let pacer = (!workout.is_empty()).then(|| {
            println!("Cues: {}", describe_workout(&workout));
            Pacer::new(workout)
        });
        if let Some(f) = setup.focus {
            println!("Focus: {} (\"{}\")", f.label(), f.cue());
        }
        Self {
            heart,
            drag_target: ctx.and_then(|c| c.profile.drag_target),
            pacer,
            technique: LiveTechnique::new(),
            focus: setup.focus.map(FocusCoach::new),
            speaker,
            screens: None,
            said: Vec::new(),
        }
    }

    /// Go: the workout's clock starts (from the row screen's Start; in
    /// `record`, at the first stroke). Before it, rowing is the warm-up.
    pub fn start(&mut self, go_us: i64) {
        if let Some(p) = self.pacer.as_mut() {
            p.start(go_us);
        }
    }

    pub fn started(&self) -> bool {
        self.pacer.as_ref().is_some_and(Pacer::started)
    }

    /// Between Go and the workout's end.
    pub fn running(&self, now_us: i64) -> bool {
        self.pacer.as_ref().is_some_and(|p| p.running(now_us))
    }

    /// Every second: the workout's call that's due (each part beginning,
    /// halfway, time), with the stage it begins for the recording.
    pub fn tick(&mut self, now_us: i64) {
        let Some(p) = self.pacer.as_mut() else { return };
        let Some(cue) = p.tick(now_us) else { return };
        let at = p.position(now_us);
        self.say(&cue.text());
        if let (Cue::Go { .. } | Cue::Rest | Cue::Time, Some(at)) = (cue, at) {
            self.said.push(EventKind::Stage { stage: at.stage, n: at.n });
        }
    }

    /// Speak, show on the screens, and keep for the recording.
    pub fn say(&mut self, text: &str) {
        println!("  » {text}");
        if let Some(s) = &self.speaker {
            s.say(text);
        }
        if let Some(hub) = &self.screens {
            live::publish(hub, &LiveMsg::Cue { text });
        }
        self.said.push(EventKind::Cue { text: text.to_owned() });
    }

    /// What was said and the stages begun since the last call, as events at
    /// `t_us` for the recording: a review can then see what each cue did,
    /// and when each part of the workout ran.
    pub fn take_said(&mut self, t_us: i64) -> Vec<Event> {
        self.said.drain(..).map(|kind| Event { t_us, kind }).collect()
    }

    /// A camera frame: buffer it, and show it on the screens.
    pub fn pose(&mut self, p: &model::Pose) {
        self.technique.frame(p);
        if let Some(hub) = &self.screens
            && let Some(joints) = self.technique.figure(p)
        {
            let guide = self.technique.guide(&joints);
            live::publish(hub, &LiveMsg::Figure { joints: &joints, guide: guide.as_ref() });
        }
    }

    pub fn publish_stroke(&self, r: &StrokeRecord) {
        let Some(hub) = &self.screens else { return };
        let (typical_catch, typical_finish) = self.technique.typical();
        let pose = r.pose.as_ref();
        let timing = analysis::technique::handle_timing(r, pose);
        live::publish(
            hub,
            &LiveMsg::Stroke(Box::new(StrokeMsg {
                index: r.index,
                watts: r.watts,
                rate: r.rate(),
                hr: r.hr_bpm,
                zone: r.hr_bpm.zip(self.heart.as_ref()).map(|(b, h)| h.zone(b)),
                faults: r.faults.iter().map(|h| h.fault.into()).collect(),
                drive_s: timing.map(|t| t.0 / 1000.0),
                recovery_s: timing.map(|t| t.1 / 1000.0),
                catch: pose.map(|p| &p.catch_figure),
                finish: pose.map(|p| &p.finish_figure),
                catch_angles: pose.map(|p| p.catch),
                finish_angles: pose.map(|p| p.finish),
                sequence: pose.map(|p| p.sequence),
                typical_catch: &typical_catch,
                typical_finish: &typical_finish,
            })),
        );
    }

    /// The focus, the targets (the current interval's, for intervals) and the
    /// good stroke on this rower's body, for screens that just connected (and
    /// as the body is measured again).
    pub fn publish_focus(&self, now_us: i64) {
        if let Some(hub) = &self.screens {
            let fault = self.focus.as_ref().map(|f| f.focus());
            live::publish(hub, &LiveMsg::Focus { fault, cue: fault.map(|f| f.cue()) });
            let pacer = self.pacer.as_ref();
            let targets = pacer.and_then(|p| p.targets(now_us)).unwrap_or_default();
            live::publish(
                hub,
                &LiveMsg::Targets {
                    targets: &targets,
                    // The pace ghost's: the pace, when heart rate has moved it.
                    rhythm: pacer.and_then(|p| p.rhythm(now_us)),
                    position: pacer.and_then(|p| p.position(now_us)),
                },
            );
            if let Some(target) = self.technique.target() {
                live::publish(hub, &LiveMsg::Target(&target));
            }
        }
    }

    /// Give a completed stroke its camera view and faults.
    pub fn observe(&mut self, r: &mut StrokeRecord) {
        self.technique.stroke(r);
    }

    /// React to a completed (observed) stroke.
    pub fn react(&mut self, r: &StrokeRecord, count: u32) {
        if count == DRAG_CHECK_STROKE
            && let Some(df) = r.drag_factor
        {
            match self.drag_target {
                Some(t) if (df as i16 - t as i16).abs() > DRAG_TOLERANCE => {
                    self.say(&format!("Drag factor {df}. Target {t}."));
                }
                Some(_) => {}
                None => println!("  Drag factor {df} (no target: set one with rowctl profile --drag {df})"),
            }
        }
        // One cue per stroke at most, pacing first: a change of pace says
        // what to row at and isn't repeated, while a technique cue it
        // displaces comes again within half a minute.
        let technique = self.focus.as_mut().and_then(|f| f.stroke(r));
        let pacing = self.pacer.as_mut().and_then(|p| p.stroke(r));
        match (pacing, technique) {
            (Some(p), _) => self.say(&p.text()),
            (None, Some(t)) => self.say(t.text()),
            (None, None) => {}
        }
    }
}

/// The database is useful for zones and saving, but recording must not
/// depend on it.
/// Typed marks become `Marker` events: what's being rowed on purpose, for
/// checking the fault detectors (`rowctl check`). Lines can also be piped in.
fn spawn_markers(clock: Clock, tx: mpsc::Sender<Event>) {
    use std::io::{BufRead, IsTerminal};
    if std::io::stdin().is_terminal() {
        let faults: Vec<String> = Fault::ALL.iter().enumerate().map(|(i, f)| format!("{} {}", i + 1, f.label())).collect();
        println!(
            "Mark what you're rowing on purpose: type a number and Enter. 0 clean, {}. Other text is saved as a note.",
            faults.join(", ")
        );
    }
    // A plain thread: reading stdin blocks, and it ends with the process.
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let numbered = |n: &str| n.parse::<usize>().ok()?.checked_sub(1).and_then(|i| Fault::ALL.get(i));
            let label = match line.trim() {
                "" => continue,
                "0" => analysis::fixtures::CLEAN.to_string(),
                other => numbered(other).map_or_else(|| other.to_string(), |f| f.label().to_owned()),
            };
            let ev = Event {
                t_us: clock.now_us(),
                kind: EventKind::Marker { label },
            };
            if tx.blocking_send(ev).is_err() {
                break;
            }
        }
    });
}

async fn print_readiness(ctx: Option<&Ctx>) {
    if let Some(c) = ctx
        && let Ok(line) = crate::health::line(c).await
    {
        println!("{line}");
    }
}

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
    header.plan = args.plan.map(|p| p.kind().name().to_owned());
    print_readiness(ctx.as_ref()).await;
    let focus = choose_focus(&args, ctx.as_ref()).await;
    header.focus = focus.map(|f| f.label().to_owned());
    let writer = FileWriter::create(&path, &header)?;
    let mut setup = args.setup(focus);
    if let (Some(c), Some(k)) = (&ctx, setup.kind) {
        setup.progression = c.progression(k).await.unwrap_or_else(|e| {
            eprintln!("Couldn't read the last {}: {e:#}", k.label());
            None
        });
    }
    let coach = Coach::new(&setup, ctx.as_ref(), args.speak.then(start_speaker).flatten());
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
    let coach = Coach::new(&args.setup(focus), ctx.as_ref(), args.speak.then(start_speaker).flatten());
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
    let tasks = match &args.replay {
        Some(file) => {
            let (_, events, _) = recording::read_all(file)?;
            println!("Replaying {} ({} events)", file.display(), events.len());
            vec![tokio::spawn(replay(events, clock, tx.clone()))]
        }
        None => spawn_devices(args.devices(), clock, &tx),
    };
    // The camera page's frames join the same stream as the devices'; the
    // mirror follows the session on /ws/live.
    let mut servers = Vec::new();
    if args.camera || args.phone {
        let hub = live::channel();
        let session = server::Live::into_session(clock, tx.clone()).with_broadcast(hub.clone());
        let api = pool.map(server::Api::new);
        if let Some(tasks) = crate::screens::start(args.listen, args.phone, session, api, &crate::default_web_dir()).await {
            coach.screens = Some(hub);
            servers = tasks;
        }
    }
    if writer.is_some() {
        spawn_markers(clock, tx.clone());
    }
    drop(tx);

    let start_us = clock.now_us();
    let mut view = View {
        heart: coach.heart,
        ..Default::default()
    };
    let mut asm = StrokeAssembler::new();
    let mut hearts = HeartPick::default();
    let mut ticker = tokio::time::interval(STATUS_EVERY);
    ticker.tick().await;
    // The row screen's countdowns move every second.
    let mut screens = tokio::time::interval(Duration::from_secs(1));
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
                if !hearts.keep(&ev) {
                    continue;
                }
                if let Some(w) = writer.as_mut() {
                    w.write(&ev)?;
                }
                if let Some(msg) = view.apply(&ev) {
                    println!("{msg}");
                }
                if let EventKind::Pose(p) = &ev.kind {
                    coach.pose(p);
                }
                if let Some(mut r) = asm.push(&ev) {
                    view.strokes += 1;
                    // No Start button here: the workout starts with the row.
                    if !coach.started() {
                        coach.start(r.t_catch_us);
                    }
                    coach.observe(&mut r);
                    println!("{}", record_line(&r, start_us, view.heart.as_ref()));
                    coach.publish_stroke(&r);
                    coach.react(&r, view.strokes);
                }
                for cue in coach.take_said(ev.t_us) {
                    if let Some(w) = writer.as_mut() {
                        w.write(&cue)?;
                    }
                }
            }
            _ = ticker.tick() => {
                println!("{}", view.status_line(clock.now_us() - start_us));
            }
            _ = screens.tick() => {
                let now = clock.now_us();
                coach.tick(now);
                for ev in coach.take_said(now) {
                    if let Some(w) = writer.as_mut() {
                        w.write(&ev)?;
                    }
                }
                coach.publish_focus(now);
            }
            _ = &mut ctrl_c => break,
            _ = &mut deadline => break,
        }
    }

    // Closing the channel stops the device tasks; give the H10 a moment to
    // disconnect cleanly so the strap is free for the PM5 or phone.
    drop(rx);
    for task in servers {
        task.abort();
    }
    for task in tasks {
        let _ = tokio::time::timeout(Duration::from_secs(3), task).await;
    }
    Ok((writer, view.strokes))
}

#[derive(Default)]
pub struct View {
    pub heart: Option<HeartProfile>,
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
    pub fn apply(&mut self, ev: &Event) -> Option<String> {
        self.events += 1;
        match &ev.kind {
            EventKind::Pm5Status(s) => self.pm5 = Some(s.clone()),
            EventKind::Heart(h) => self.bpm = Some(h.bpm),
            EventKind::Pose(_) => self.poses += 1,
            EventKind::Marker { label } => return Some(format!("● marked: {label}")),
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


#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_replay_happens_now_at_its_own_pace_without_what_the_coach_said() {
        // A recording from long ago: 30 ms of it.
        let then = 1_000_000_000;
        let mark = |t_us: i64, label: &str| Event {
            t_us,
            kind: EventKind::Marker { label: label.into() },
        };
        let pose = model::Pose {
            captured_us: then + 5_000,
            width: 1280,
            height: 720,
            points: Vec::new(),
        };
        let events = vec![
            mark(then, "first"),
            Event {
                t_us: then + 10_000,
                kind: EventKind::Pose(pose),
            },
            Event {
                t_us: then + 15_000,
                kind: EventKind::Cue { text: "Slow the slide".into() },
            },
            mark(then + 30_000, "last"),
        ];
        let clock = Clock::start();
        let (tx, mut rx) = mpsc::channel(8);
        let before = clock.now_us();
        replay(events, clock, tx).await;
        let took = clock.now_us() - before;
        let got: Vec<Event> = std::iter::from_fn(|| rx.try_recv().ok()).collect();

        assert_eq!(got.len(), 3, "the cue is this session's coach's to give");
        assert!(got[0].t_us >= before, "moved to now");
        assert_eq!(got[2].t_us - got[0].t_us, 30_000, "spaced as recorded");
        assert!(took >= 30_000, "and played at that pace: {took} µs");
        // The camera's frame keeps its place among the PM5's events.
        let EventKind::Pose(p) = &got[1].kind else { panic!("{:?}", got[1]) };
        assert_eq!(got[1].t_us - p.captured_us, 5_000);
    }
}
