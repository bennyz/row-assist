//! `serve`: Row Assist as one long-running server, started at login.
//!
//! The page at http://localhost:8080 shows today's plan and the devices. The
//! devices stay connected; a row records from its first stroke and ends by
//! itself after `--end-after` seconds without one (or with Stop on the page).
//! Then it's analysed and saved, the page shows its review, and the next row
//! is planned. Watch data fetched while waiting re-plans at once.
//!
//! Besides the devices, a recording gets what the coach said, what the rower
//! marked as rowed on purpose (from the row screen), and the plan and focus
//! in its header; camera stills go in a folder beside it.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use analysis::strokes::StrokeAssembler;
use anyhow::{Context, Result};
use clap::Args;
use devices::sim;
use model::{Clock, Device, Event, EventKind};
use recording::{FileWriter, Header};
use server::live::{self, LiveMsg, LiveSender};
use server::row::{self, Command, DeviceLink, Ended, Mark, Phase, RowState};
use tokio::sync::{Notify, mpsc, watch};

use crate::ctx::{Ctx, today};
use crate::live::{Coach, Devices, HeartPick, Setup, View, spawn_devices, start_speaker};
use crate::plan::describe_targets;
use crate::planning::{setup_for, today_plan};
use crate::render::record_line;
use crate::review;
use crate::speak::Speaker;

/// Events kept from before the first stroke, so the recording has its start.
const PRE_ROLL_US: i64 = 20_000_000;
/// Fewer strokes than this is a bump of the handle, not a row: not kept.
pub const MIN_STROKES: u32 = 10;
const TICK: Duration = Duration::from_secs(1);
/// No beat for this long: the strap is off, whichever way it came in.
const HEART_STALE_US: i64 = 5_000_000;
/// A camera still this long after the first stroke, then every
/// `STILL_EVERY_US`: at 20 spm that lands on every part of the stroke over
/// a row, about 60 small pictures in 20 minutes.
const FIRST_STILL_US: i64 = 5_000_000;
const STILL_EVERY_US: i64 = 20_000_000;

#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Address for the pages; the phone and the Watch upload reach it over
    /// the network, everything else only from this computer
    #[arg(long, default_value = "0.0.0.0:8080")]
    addr: SocketAddr,
    /// End and save a row after this many seconds without a stroke
    #[arg(long, default_value_t = 120)]
    end_after: u64,
    /// A simulated rower, with a camera, instead of the PM5, H10 and phone
    #[arg(long)]
    sim: bool,
    /// Only the review screen: no devices, nothing recorded
    #[arg(long)]
    review_only: bool,
    /// Connect to the heart-rate strap whose name contains this
    #[arg(long, default_value = "H10")]
    h10_name: String,
    #[arg(long)]
    db: Option<PathBuf>,
    /// Built frontend [default: frontend/dist in this checkout]
    #[arg(long, env = "ROW_ASSIST_WEB")]
    web: Option<PathBuf>,
}

pub async fn run(args: ServeArgs) -> Result<()> {
    let web = args.web.clone().unwrap_or_else(crate::default_web_dir);
    if args.review_only {
        return server::serve(server::Config {
            addr: args.addr,
            db: args.db.unwrap_or_else(store::paths::db_path),
            web_dir: web,
        })
        .await;
    }
    let ctx = Ctx::open(args.db.clone()).await?;
    let clock = Clock::start();
    let (tx, mut events) = mpsc::channel::<Event>(4096);
    let devices = Devices {
        sim: args.sim.then(|| sim::SimConfig {
            pose: Some(Default::default()),
            ..Default::default()
        }),
        pm5: true,
        h10: Some(args.h10_name.clone()),
    };
    let device_tasks = spawn_devices(devices, clock, &tx);

    let hub = live::channel();
    // Watch data comes from the home server, when a link to it is set.
    let watch_data = Arc::new(Notify::new());
    if let Some(link) = store::health::source(&store::paths::health_source_path())? {
        tokio::spawn(server::health::keep_synced(ctx.pool.clone(), link, watch_data.clone()));
    }
    let (state_tx, state) = watch::channel(RowState::default());
    let (commands, mut command_rx) = mpsc::channel(8);
    let mut api = server::Api::new(ctx.pool.clone());
    api.row = Some(row::Control { state, commands });
    let session = server::Live::into_session(clock, tx.clone()).with_broadcast(hub.clone());
    let camera = session.clone();
    // The simulator is the camera with --sim; otherwise a phone.
    let servers = crate::screens::start(args.addr, !args.sim, session, Some(api), &web)
        .await
        .context("couldn't serve the pages")?;
    drop(tx);
    println!("Row Assist: http://localhost:{} (start rowing and it records)", args.addr.port());

    let sessions = store::paths::sessions_dir();
    let mut runner = Runner::new(ctx, clock, hub, state_tx, start_speaker(), args.sim, args.end_after, sessions).await;
    runner.camera = (!args.sim).then_some(camera);
    let mut tick = tokio::time::interval(TICK);
    let quit = shutdown();
    tokio::pin!(quit);
    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Some(ev) => runner.event(ev),
                None => break,
            },
            _ = tick.tick() => runner.tick(clock.now_us()).await,
            _ = watch_data.notified() => runner.watch_data().await,
            Some(c) = command_rx.recv() => match c {
                Command::Stop => runner.end().await,
                Command::Mark(m) => runner.mark(m),
            },
            _ = &mut quit => break,
        }
    }
    // A row in progress when the laptop shuts down is saved.
    runner.end().await;
    for task in servers {
        task.abort();
    }
    drop(events);
    for task in device_tasks {
        let _ = tokio::time::timeout(Duration::from_secs(3), task).await;
    }
    Ok(())
}

/// Ask the camera page for a still and save it as `<asked at, µs>.jpg` in the
/// row's stills folder. The page draws the frame it has when the request
/// arrives, a few milliseconds after `t_us`.
async fn save_still(camera: server::Live, rowrec: PathBuf, t_us: i64) {
    let Some(jpeg) = camera.still().await else { return };
    let saved = tokio::task::spawn_blocking(move || {
        // A bump of the handle: its recording was deleted while the phone answered.
        if !rowrec.exists() {
            return Ok(());
        }
        let dir = recording::stills_dir(&rowrec);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(format!("{t_us}.jpg")), &jpeg)
    })
    .await;
    if let Ok(Err(e)) = saved {
        eprintln!("Couldn't save a camera still: {e}");
    }
}

/// Ctrl-C, or systemd stopping the service.
async fn shutdown() {
    #[cfg(unix)]
    if let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// The row being recorded.
struct Rec {
    writer: FileWriter,
    path: PathBuf,
    /// The first stroke's catch.
    started_us: i64,
    last_stroke_us: i64,
    next_still_us: i64,
}

struct Runner {
    ctx: Ctx,
    clock: Clock,
    hub: LiveSender,
    state_tx: watch::Sender<RowState>,
    speaker: Option<Speaker>,
    sim: bool,
    end_after_us: i64,
    sessions: PathBuf,
    state: RowState,
    /// What the next (or current) row is set up as, and the day it was planned.
    setup: Setup,
    plan_day: i64,
    coach: Coach,
    asm: StrokeAssembler,
    view: View,
    /// While waiting: the last `PRE_ROLL_US` of events.
    pre: VecDeque<Event>,
    rec: Option<Rec>,
    hearts: HeartPick,
    /// The strap's own connection to this machine, as its driver reports it.
    h10_link: DeviceLink,
    /// The last beat: when, and whether it came through the PM5.
    heart_at: Option<(i64, bool)>,
    /// The phone's camera page, for stills during a row.
    camera: Option<server::Live>,
}

impl Runner {
    #[allow(clippy::too_many_arguments)]
    async fn new(
        ctx: Ctx,
        clock: Clock,
        hub: LiveSender,
        state_tx: watch::Sender<RowState>,
        speaker: Option<Speaker>,
        sim: bool,
        end_after_s: u64,
        sessions: PathBuf,
    ) -> Self {
        let coach = Coach::new(&Setup::default(), None, None);
        let mut r = Self {
            ctx,
            clock,
            hub,
            state_tx,
            speaker,
            sim,
            end_after_us: end_after_s as i64 * 1_000_000,
            sessions,
            state: RowState {
                ends_after_s: end_after_s as f64,
                ..Default::default()
            },
            setup: Setup::default(),
            plan_day: 0,
            coach,
            asm: StrokeAssembler::new(),
            view: View::default(),
            pre: VecDeque::new(),
            rec: None,
            hearts: HeartPick::default(),
            h10_link: DeviceLink::default(),
            heart_at: None,
            camera: None,
        };
        r.replan().await;
        r.publish();
        r
    }

    /// Plan the next row from everything so far. Only between rows.
    async fn replan(&mut self) {
        if let Err(e) = self.ctx.refresh().await {
            eprintln!("Couldn't reload the profile: {e:#}");
        }
        let day = today();
        // From the page, assume the camera will be on: it's set up there.
        let (proposal, readiness) = match today_plan(&self.ctx, true, day).await {
            Ok(x) => x,
            Err(e) => {
                eprintln!("Couldn't plan today's row: {e:#}");
                return;
            }
        };
        println!("Next row: {}", proposal.kind.describe());
        self.setup = setup_for(&proposal);
        let heart = self.ctx.heart().map(|h| h.0);
        self.state.plan = Some(row::Plan {
            pm5_setup: proposal.pm5_setup(),
            cues: describe_targets(&self.setup.targets(heart.as_ref()).0),
            proposal,
            readiness,
        });
        self.plan_day = day;
        self.coach = Coach::new(&self.setup, Some(&self.ctx), self.speaker.clone());
        self.coach.screens = Some(self.hub.clone());
    }

    fn event(&mut self, ev: Event) {
        if !self.hearts.keep(&ev) {
            return;
        }
        let t_us = ev.t_us;
        if let Some(msg) = self.view.apply(&ev) {
            println!("{msg}");
        }
        match &ev.kind {
            EventKind::Link(l) => {
                let slot = match l.device {
                    Device::Pm5 => &mut self.state.devices.pm5,
                    Device::H10 => &mut self.h10_link,
                    Device::Camera => &mut self.state.devices.camera,
                };
                *slot = DeviceLink {
                    connected: Some(l.connected),
                    detail: l.detail.clone(),
                };
            }
            EventKind::Heart(h) => {
                self.state.hr = Some(h.bpm);
                self.heart_at = Some((ev.t_us, h.via_pm5));
            }
            EventKind::Pose(p) => self.coach.pose(p),
            _ => {}
        }
        let stroke = self.asm.push(&ev);
        match &mut self.rec {
            Some(rec) => {
                if let Err(e) = rec.writer.write(&ev) {
                    eprintln!("Recording to {} failed: {e:#}", rec.path.display());
                }
            }
            None => {
                let t = ev.t_us;
                self.pre.push_back(ev);
                while self.pre.front().is_some_and(|e| e.t_us < t - PRE_ROLL_US) {
                    self.pre.pop_front();
                }
            }
        }
        let Some(mut r) = stroke else { return };
        if self.rec.is_none() && !self.begin(r.t_catch_us) {
            return;
        }
        self.state.strokes += 1;
        let Some(rec) = self.rec.as_mut() else { return };
        rec.last_stroke_us = r.t_next_catch_us.unwrap_or(r.t_catch_us);
        let started = rec.started_us;
        self.coach.observe(&mut r);
        println!("{}", record_line(&r, started, self.coach.heart.as_ref()));
        self.coach.publish_stroke(&r);
        self.coach.react(&r, self.state.strokes);
        for cue in self.coach.take_said(t_us) {
            self.write(&cue);
        }
    }

    /// Into the recording, when there is one.
    fn write(&mut self, ev: &Event) {
        if let Some(rec) = self.rec.as_mut()
            && let Err(e) = rec.writer.write(ev)
        {
            eprintln!("Recording to {} failed: {e:#}", rec.path.display());
        }
    }

    /// From the row screen: what's rowed on purpose from now on. Before the
    /// first stroke it waits in the pre-roll, so the row starts marked.
    fn mark(&mut self, m: Mark) {
        println!("Marked: {}", m.label());
        self.state.mark = Some(m);
        let ev = Event {
            t_us: self.clock.now_us(),
            kind: EventKind::Marker { label: m.label().to_owned() },
        };
        self.event(ev);
        self.publish();
    }

    /// The first stroke: start the recording with what led up to it.
    fn begin(&mut self, first_catch_us: i64) -> bool {
        let start_us = self.pre.front().map_or(first_catch_us, |e| e.t_us);
        let name = crate::ctx::local(start_us)
            .map(|t| t.format("%Y%m%d-%H%M%S").to_string())
            .unwrap_or_else(|| start_us.to_string());
        let path = self.sessions.join(format!("{name}.rowrec"));
        let mut header = Header::new(start_us, concat!("rowctl ", env!("CARGO_PKG_VERSION")));
        header.plan = self.setup.plan.map(|p| p.name().to_owned());
        header.focus = self.setup.focus.map(|f| f.label().to_owned());
        header.simulated = self.sim;
        let mut writer = match FileWriter::create(&path, &header) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("Can't record ({e:#})");
                return false;
            }
        };
        // A mark made longer ago than the pre-roll still holds.
        let marked = self.pre.iter().any(|e| matches!(e.kind, EventKind::Marker { .. }));
        let mark = self.state.mark.filter(|_| !marked).map(|m| Event {
            t_us: start_us,
            kind: EventKind::Marker { label: m.label().to_owned() },
        });
        for ev in mark.into_iter().chain(self.pre.drain(..)) {
            if let Err(e) = writer.write(&ev) {
                eprintln!("Recording to {} failed: {e:#}", path.display());
                break;
            }
        }
        println!("Rowing: recording to {}", path.display());
        self.rec = Some(Rec {
            writer,
            path,
            started_us: first_catch_us,
            last_stroke_us: first_catch_us,
            next_still_us: first_catch_us + FIRST_STILL_US,
        });
        self.state.phase = Phase::Rowing;
        self.publish();
        true
    }

    async fn tick(&mut self, now_us: i64) {
        match &self.rec {
            Some(rec) if now_us - rec.last_stroke_us >= self.end_after_us => {
                println!("No strokes for {:.0} s: ending the row", self.state.ends_after_s);
                self.end().await;
            }
            None if today() != self.plan_day => self.replan().await,
            _ => {}
        }
        self.ask_for_still(now_us);
        self.coach.publish_focus();
        self.publish_at(now_us);
    }

    /// Every so often during a row, a still from the camera page, saved
    /// beside the recording.
    fn ask_for_still(&mut self, now_us: i64) {
        let camera_on = self.state.devices.camera.connected == Some(true);
        if let (Some(rec), Some(camera), true) = (self.rec.as_mut(), &self.camera, camera_on)
            && now_us >= rec.next_still_us
        {
            rec.next_still_us = now_us + STILL_EVERY_US;
            tokio::spawn(save_still(camera.clone(), rec.path.clone(), now_us));
        }
    }

    async fn watch_data(&mut self) {
        println!("Watch data received");
        self.state.watch_at_us = Some(self.clock.now_us());
        // Mid-row, it waits for the next plan: zones don't change under a row.
        if self.rec.is_none() {
            self.replan().await;
        }
        self.publish();
    }

    /// End the row (if there is one): save it, or drop it if it was a bump.
    async fn end(&mut self) {
        let Some(rec) = self.rec.take() else { return };
        self.state.phase = Phase::Saving;
        self.publish();
        let strokes = self.state.strokes;
        let events = rec.writer.events();
        let path = rec.path;
        let session = match rec.writer.finish() {
            Err(e) => {
                eprintln!("Couldn't finish {}: {e:#}", path.display());
                None
            }
            Ok(_) if strokes < MIN_STROKES => {
                println!("{strokes} strokes: not a row, not kept");
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::remove_dir_all(recording::stills_dir(&path));
                None
            }
            Ok(_) => {
                println!("\nSaved {events} events, {strokes} strokes to {}\n", path.display());
                match review::process(&self.ctx, &path, true).await {
                    Ok(done) => {
                        let _ = done.print(&self.ctx).await;
                        done.saved
                    }
                    Err(e) => {
                        eprintln!("Analysis failed ({e:#}); retry with: rowctl analyze {}", path.display());
                        None
                    }
                }
            }
        };
        self.state.last = Some(Ended {
            at_us: self.clock.now_us(),
            strokes,
            session,
        });
        self.state.strokes = 0;
        self.state.mark = None;
        self.asm = StrokeAssembler::new();
        self.pre.clear();
        self.state.phase = Phase::Ready;
        // The row just saved counts in the next plan.
        self.replan().await;
        self.publish();
    }

    fn publish(&mut self) {
        self.publish_at(self.clock.now_us());
    }

    /// To the page: on `/ws/live`, and for `GET /api/row`.
    fn publish_at(&mut self, now_us: i64) {
        let secs = |us: i64| Some((us as f64 / 1e6).max(0.0));
        (self.state.elapsed_s, self.state.idle_s) = match &self.rec {
            Some(rec) => (secs(now_us - rec.started_us), secs(now_us - rec.last_stroke_us)),
            None => (None, None),
        };
        // The Heart chip: the strap's own link, or the PM5 passing it on.
        let beating = self.heart_at.filter(|&(t, _)| now_us - t < HEART_STALE_US);
        if beating.is_none() {
            self.state.hr = None;
        }
        self.state.devices.h10 = match beating {
            Some((_, true)) if self.h10_link.connected != Some(true) => DeviceLink {
                connected: Some(true),
                detail: Some("through the PM5".into()),
            },
            _ => self.h10_link.clone(),
        };
        live::publish(&self.hub, &LiveMsg::Row(&self.state));
        self.state_tx.send_replace(self.state.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000_000;

    async fn runner(name: &str) -> (Runner, watch::Receiver<RowState>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("row-assist-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ctx = Ctx::open(Some(dir.join("t.db"))).await.unwrap();
        let (state_tx, state) = watch::channel(RowState::default());
        let r = Runner::new(ctx, Clock::start(), live::channel(), state_tx, None, true, 120, dir.join("sessions")).await;
        (r, state, dir)
    }

    fn rowing(start_us: i64, seconds: u64) -> Vec<Event> {
        sim::generate(sim::SimConfig::default(), start_us, Duration::from_secs(seconds))
    }

    fn files(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir.join("sessions")).map_or(0, |d| d.count())
    }

    #[tokio::test]
    async fn a_row_records_from_its_first_stroke_and_saves_when_rowing_stops() {
        let (mut r, state, dir) = runner("row").await;
        assert_eq!(state.borrow().phase, Phase::Ready);
        let plan = state.borrow().plan.clone().expect("a plan before the first row");
        assert_eq!(plan.proposal.kind, analysis::planner::Kind::Reference, "no rows yet: start with the reference");
        assert_eq!(r.setup.plan, Some(crate::plan::Plan::Reference));

        let t0 = Clock::start().now_us();
        let events = rowing(t0, 90);
        let end = events.last().unwrap().t_us;
        for ev in events {
            r.event(ev);
        }
        r.tick(end).await;
        assert_eq!(state.borrow().phase, Phase::Rowing);
        let strokes = state.borrow().strokes;
        assert!(strokes >= 25, "{strokes} strokes in 90 s at rate 20");

        r.tick(end + MIN).await;
        assert_eq!(state.borrow().phase, Phase::Rowing, "a minute's rest isn't the end");
        assert!(state.borrow().idle_s.unwrap() >= 59.0);
        r.tick(end + 2 * MIN + 1).await;
        let s = state.borrow().clone();
        assert_eq!(s.phase, Phase::Ready);
        let last = s.last.expect("ended");
        assert_eq!(last.strokes, strokes);
        let id = last.session.expect("saved");
        assert_eq!(files(&dir), 1);

        let saved = store::sessions::list_sessions(&r.ctx.pool).await.unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, id);
        assert_eq!(saved[0].meta.plan.as_deref(), Some("reference"));
        assert!(saved[0].meta.simulated);
        let (header, recorded, _) = recording::read_all(std::path::Path::new(&saved[0].meta.recording)).unwrap();
        assert!(header.started_at_us <= t0 + 3_000_000, "the recording has what led up to the first stroke");
        assert!(recorded.iter().any(|e| matches!(e.kind, EventKind::Link(_))));

        // A few strokes, then nothing: a bump of the handle, not kept.
        let t1 = end + 10 * MIN;
        let bump = rowing(t1, 12);
        let bump_end = bump.last().unwrap().t_us;
        for ev in bump {
            r.event(ev);
        }
        r.tick(bump_end + 2 * MIN + 1).await;
        let last = state.borrow().last.clone().unwrap();
        assert!(last.strokes < MIN_STROKES && last.session.is_none(), "{last:?}");
        assert_eq!(files(&dir), 1, "the bump's file is gone");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn stop_ends_the_row_at_once_and_does_nothing_between_rows() {
        let (mut r, state, dir) = runner("stop").await;
        r.end().await;
        assert!(state.borrow().last.is_none(), "nothing to end");
        for ev in rowing(Clock::start().now_us(), 60) {
            r.event(ev);
        }
        r.end().await;
        let s = state.borrow().clone();
        assert_eq!(s.phase, Phase::Ready);
        assert!(s.last.unwrap().session.is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn the_recording_has_what_was_marked_and_what_the_coach_said() {
        let (mut r, state, dir) = runner("marks").await;
        // A drag factor far from the simulator's, so the coach says so.
        let profile = store::sessions::Profile {
            drag_target: Some(150),
            ..Default::default()
        };
        store::sessions::set_profile(&r.ctx.pool, &profile).await.unwrap();
        r.replan().await;

        // Marked well before the first stroke: longer ago than the pre-roll.
        r.mark(Mark {
            fault: Some(analysis::technique::Fault::EarlyArms),
        });
        assert_eq!(state.borrow().mark.map(|m| m.label()), Some("early arms"));
        let t0 = Clock::start().now_us() + 2 * PRE_ROLL_US;
        let events = rowing(t0, 60);
        let end = events.last().unwrap().t_us;
        for ev in events {
            r.event(ev);
        }
        r.tick(end + 2 * MIN + 1).await;
        assert_eq!(state.borrow().mark, None, "the next row starts unmarked");

        let saved = store::sessions::list_sessions(&r.ctx.pool).await.unwrap();
        let (_, recorded, _) = recording::read_all(std::path::Path::new(&saved[0].meta.recording)).unwrap();
        let marks: Vec<_> = recorded
            .iter()
            .filter_map(|e| match &e.kind {
                EventKind::Marker { label } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(marks, ["early arms"]);
        let blocks = analysis::fixtures::blocks(&recorded);
        assert_eq!(blocks.len(), 1, "rowctl check sees the block");
        assert!(blocks[0].from_us <= recorded[1].t_us, "from the start of the recording");
        let cues: Vec<_> = recorded
            .iter()
            .filter_map(|e| match &e.kind {
                EventKind::Cue { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(cues.iter().any(|c| c.contains("Target 150")), "{cues:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn heart_rate_comes_through_the_pm5_unless_the_strap_is_connected_here() {
        let (mut r, state, dir) = runner("heart").await;
        let beat = |t_us: i64, bpm: u16, via_pm5: bool| Event {
            t_us,
            kind: EventKind::Heart(model::Heart {
                bpm,
                via_pm5,
                ..Default::default()
            }),
        };
        let h10 = |t_us: i64, connected: bool| Event {
            t_us,
            kind: EventKind::Link(model::Link {
                device: Device::H10,
                connected,
                detail: None,
            }),
        };
        let t = Clock::start().now_us();

        r.event(beat(t, 84, true));
        r.tick(t).await;
        let s = state.borrow().clone();
        assert_eq!((s.hr, s.devices.h10.connected), (Some(84), Some(true)));
        assert_eq!(s.devices.h10.detail.as_deref(), Some("through the PM5"));

        // The strap connects here too: the PM5's copy of each beat is dropped.
        r.event(h10(t + 1_000_000, true));
        r.event(beat(t + 1_100_000, 90, false));
        r.event(beat(t + 1_200_000, 70, true));
        r.tick(t + 1_300_000).await;
        let s = state.borrow().clone();
        assert_eq!(s.hr, Some(90));
        assert_eq!(s.devices.h10.detail, None, "the strap's own link");

        // It drops from here: the PM5 again.
        r.event(h10(t + 2_000_000, false));
        r.event(beat(t + 2_100_000, 88, true));
        r.tick(t + 2_200_000).await;
        assert_eq!(state.borrow().hr, Some(88));

        // No beats at all for a while: no number, and the chip says so.
        r.tick(t + 2_100_000 + HEART_STALE_US).await;
        let s = state.borrow().clone();
        assert_eq!((s.hr, s.devices.h10.connected), (None, Some(false)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
