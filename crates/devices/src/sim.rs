//! Simulated PM5 + heart-rate strap, for working without hardware.
//!
//! Emits the same event shapes as the real drivers at the same rates: a status
//! poll every 50 ms, force chunks during the drive, one heart notification a
//! second. Deterministic (fixed-seed PRNG) so test runs are repeatable.

use std::f64::consts::PI;
use std::time::Duration;

use model::{Clock, Device, Event, EventKind, Heart, Link, Pm5Force, Pm5Status, StrokeState};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct SimConfig {
    pub spm: f64,
    pub watts: f64,
    pub drag_factor: u8,
    /// Fraction of the stroke cycle spent driving (~0.33 is a 1:2 ratio).
    pub drive_fraction: f64,
    pub peak_force_lbf: f64,
    pub hr_start: f64,
    pub hr_end: f64,
    /// Seconds for heart rate to climb from start to end.
    pub hr_ramp_s: f64,
    pub seed: u64,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            spm: 20.0,
            watts: 83.0,
            drag_factor: 110,
            drive_fraction: 0.33,
            peak_force_lbf: 120.0,
            hr_start: 110.0,
            hr_end: 145.0,
            hr_ramp_s: 300.0,
            seed: 0x5EED,
        }
    }
}

/// Time between status polls, as the real driver polls.
pub const POLL: Duration = Duration::from_millis(50);
/// Force samples produced per poll during the drive.
const POINTS_PER_POLL: usize = 2;

/// The simulated erg and strap, advanced one poll at a time.
pub struct Sim {
    cfg: SimConfig,
    rng: XorShift,
    cycle_s: f64,
    drive_s: f64,
    /// m/s, from P = 2.8 v^3.
    speed: f64,
    distance_m: f64,
    next_hr_s: f64,
    peak_scale: f64,
    last_cycle: u64,
}

impl Sim {
    pub fn new(cfg: SimConfig) -> Self {
        let cycle_s = 60.0 / cfg.spm;
        Self {
            rng: XorShift(cfg.seed.max(1)),
            drive_s: cycle_s * cfg.drive_fraction,
            speed: (cfg.watts / 2.80).cbrt(),
            cycle_s,
            distance_m: 0.0,
            next_hr_s: 0.0,
            peak_scale: 1.0,
            last_cycle: u64::MAX,
            cfg,
        }
    }

    /// Events for poll `n`, i.e. `n × POLL` into the row.
    pub fn step(&mut self, n: u64) -> Vec<EventKind> {
        let cfg = &self.cfg;
        let t = n as f64 * POLL.as_secs_f64();
        self.distance_m += self.speed * POLL.as_secs_f64();

        let cycle = (t / self.cycle_s) as u64;
        if cycle != self.last_cycle {
            self.last_cycle = cycle;
            self.peak_scale = 1.0 + 0.08 * self.rng.signed();
        }
        let in_cycle = t - cycle as f64 * self.cycle_s;
        let state = if in_cycle < self.drive_s {
            StrokeState::Drive
        } else if in_cycle < self.drive_s + 0.05 {
            StrokeState::Dwell
        } else {
            StrokeState::Recovery
        };

        let mut out = vec![EventKind::Pm5Status(Pm5Status {
            stroke_state: Some(state),
            spm: Some(cfg.spm.round() as u16),
            watts: Some((cfg.watts * self.peak_scale).round() as u16),
            distance_dm: Some((self.distance_m * 10.0) as u32),
            work_time_cs: Some((t * 100.0) as u32),
            drag_factor: Some(cfg.drag_factor),
            workout_state: Some(1),
        })];

        if state == StrokeState::Drive {
            let points = (0..POINTS_PER_POLL)
                .map(|k| {
                    let x = (in_cycle + k as f64 * POLL.as_secs_f64() / POINTS_PER_POLL as f64) / self.drive_s;
                    let f = cfg.peak_force_lbf * self.peak_scale * (PI * x.clamp(0.0, 1.0)).sin().powf(1.3);
                    (f + 2.0 * self.rng.signed()).max(0.0).round() as u16
                })
                .collect();
            out.push(EventKind::Pm5Force(Pm5Force {
                stroke_state: Some(state),
                points,
            }));
        }

        if t >= self.next_hr_s {
            self.next_hr_s += 1.0;
            let ramp = (t / cfg.hr_ramp_s).min(1.0);
            let bpm = cfg.hr_start + (cfg.hr_end - cfg.hr_start) * ramp + 1.5 * self.rng.signed();
            out.push(EventKind::Heart(Heart {
                bpm: bpm.round() as u16,
                rr_1024: vec![(60.0 / bpm * 1024.0).round() as u16],
                contact: Some(true),
            }));
        }
        out
    }
}

fn links() -> impl Iterator<Item = EventKind> {
    [Device::Pm5, Device::H10].into_iter().map(|device| {
        EventKind::Link(Link {
            device,
            connected: true,
            detail: Some("simulated".into()),
        })
    })
}

/// Stream simulated events in real time until `tx` is closed. Events are
/// stamped with the real clock, like the real drivers'.
pub async fn run(clock: Clock, tx: mpsc::Sender<Event>, cfg: SimConfig) {
    for kind in links() {
        if tx.send(Event { t_us: clock.now_us(), kind }).await.is_err() {
            return;
        }
    }
    let mut sim = Sim::new(cfg);
    let mut tick = tokio::time::interval(POLL);
    for n in 0u64.. {
        tick.tick().await;
        let t_us = clock.now_us();
        for kind in sim.step(n) {
            if tx.send(Event { t_us, kind }).await.is_err() {
                return;
            }
        }
    }
}

/// A whole simulated session at once, stamped with simulated time from `start_us`.
pub fn generate(cfg: SimConfig, start_us: i64, duration: Duration) -> Vec<Event> {
    let mut out: Vec<Event> = links().map(|kind| Event { t_us: start_us, kind }).collect();
    let mut sim = Sim::new(cfg);
    let polls = (duration.as_micros() / POLL.as_micros()) as u64;
    for n in 0..polls {
        let t_us = start_us + (n as u128 * POLL.as_micros()) as i64;
        out.extend(sim.step(n).into_iter().map(|kind| Event { t_us, kind }));
    }
    out
}

struct XorShift(u64);

impl XorShift {
    /// Uniform in [-1, 1).
    fn signed(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prng_stays_in_range() {
        let mut r = XorShift(42);
        for _ in 0..10_000 {
            let v = r.signed();
            assert!((-1.0..1.0).contains(&v));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn emits_strokes_and_heart_rate() {
        let (tx, mut rx) = mpsc::channel(1024);
        let task = tokio::spawn(run(Clock::start(), tx, SimConfig::default()));
        let mut drives = 0;
        let mut hearts = 0;
        let mut last = None;
        for _ in 0..400 {
            let ev = rx.recv().await.unwrap();
            match ev.kind {
                EventKind::Pm5Status(s) => {
                    if s.stroke_state == Some(StrokeState::Drive) && last != Some(StrokeState::Drive) {
                        drives += 1;
                    }
                    last = s.stroke_state;
                }
                EventKind::Heart(_) => hearts += 1,
                _ => {}
            }
        }
        drop(rx);
        task.await.unwrap();
        // ~400 events ≈ 14 s simulated at 20 spm: about 5 drives, 14 beats.
        assert!(drives >= 4, "drives={drives}");
        assert!(hearts >= 10, "hearts={hearts}");
    }

    #[test]
    fn generate_matches_the_live_shape() {
        let evs = generate(SimConfig::default(), 1_000, Duration::from_secs(30));
        assert!(matches!(evs[0].kind, EventKind::Link(_)));
        assert!(evs.windows(2).all(|w| w[0].t_us <= w[1].t_us));
        let states: Vec<_> = evs
            .iter()
            .filter_map(|e| match &e.kind {
                EventKind::Pm5Status(s) => s.stroke_state,
                _ => None,
            })
            .collect();
        let starts = states.windows(2).filter(|w| w[1] == StrokeState::Drive && w[0] != StrokeState::Drive);
        let drives = starts.count() + usize::from(states[0] == StrokeState::Drive);
        assert_eq!(drives, 10, "30 s at 20 spm");
        let hearts = evs.iter().filter(|e| matches!(e.kind, EventKind::Heart(_))).count();
        assert_eq!(hearts, 30);
        assert_eq!(evs.last().unwrap().t_us, 1_000 + 29_950_000);
    }
}
