//! Concept2 PM5 over USB HID.
//!
//! The PM5 enumerates as a HID device (VID 0x17a4). CSAFE frames travel in
//! report 0x02 (120 bytes) on the vendor-specific interface (usage page 0xFF00).
//! hidapi is blocking, so [`run_blocking`] runs on its own thread.
//!
//! Linux needs hidraw access, e.g. `contrib/udev/50-concept2.rules`.
//!
//! Polling alternates two frames, each carrying the stroke state, so stroke
//! timing is sampled at the full poll rate:
//! - status: cadence, power, drag factor, distance, time, workout state
//! - force: up to 16 force-curve points drained from the PM's buffer

use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use hidapi::{HidApi, HidDevice};
use model::{Clock, Device, Event, EventKind, Link, Pm5Force, Pm5Status, StrokeState};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::csafe::{self, Cmd, PrevFrame, Response, le_u16, le_u32, pm_cmd, std_cmd};

pub const CONCEPT2_VID: u16 = 0x17a4;

/// Report ID 2 is the 120-byte CSAFE channel. Reads come back with the report
/// ID as the first byte; writes are `[0x02, frame..., zero padding]`.
const REPORT_ID: u8 = 0x02;
const HID_REPORT: usize = 120;
const READ_TIMEOUT_MS: i32 = 200;
/// The PM5 sends unsolicited reports on the same interface, so a reply may
/// take a few reads to arrive.
const MAX_READS: usize = 4;
/// Consecutive unanswered polls before reconnecting.
const MAX_MISSES: u32 = 200;
/// Force-plot bytes requested per poll: 32 bytes = 16 points.
const FORCE_BLOCK: u8 = 32;

pub struct Pm5 {
    dev: HidDevice,
    pub path: String,
}

/// One request/response round trip.
#[derive(Debug)]
pub struct Exchange {
    pub tx: Vec<u8>,
    /// The last raw frame read (with any leading bytes stripped).
    pub rx: Vec<u8>,
    pub response: Option<Response>,
}

impl Pm5 {
    /// Open the CSAFE interface, preferring usage page 0xFF00 / interface 1.
    pub fn open() -> Result<Self> {
        let api = HidApi::new().context("hidapi init")?;
        let devices: Vec<_> = api
            .device_list()
            .filter(|d| d.vendor_id() == CONCEPT2_VID)
            .collect();
        for d in &devices {
            debug!(
                "PM5 interface: path={:?} usage_page={:#06x} interface={}",
                d.path(),
                d.usage_page(),
                d.interface_number()
            );
        }
        let info = devices
            .iter()
            .find(|d| d.usage_page() == 0xFF00)
            .or_else(|| devices.iter().find(|d| d.interface_number() == 1))
            .or_else(|| devices.first())
            .ok_or_else(|| anyhow!("no Concept2 device on USB"))?;
        let path = info.path().to_string_lossy().into_owned();
        let dev = info
            .open_device(&api)
            .with_context(|| format!("open {path} (check the udev rule)"))?;
        Ok(Self { dev, path })
    }

    /// Send `cmds` in one frame and read until the reply to it arrives.
    ///
    /// A reply matches when its first item answers our first command, or when
    /// it carries no items at all (the PM rejected the frame and only sent a
    /// status byte). `response` is `None` if nothing matched.
    pub fn transact(&self, cmds: &[Cmd]) -> Result<Exchange> {
        let tx = csafe::encode(&csafe::payload(cmds));
        let expect = cmds.first().map(Cmd::key);
        self.write(&tx)?;
        let mut rx = Vec::new();
        for _ in 0..MAX_READS {
            let raw = self.read()?;
            if raw.is_empty() {
                continue;
            }
            if let Ok(payload) = csafe::decode(&raw)
                && let Ok(resp) = csafe::parse_response(&payload)
                && (resp.items.is_empty() || resp.items.first().map(|i| i.key()) == expect)
            {
                return Ok(Exchange { tx, rx: raw, response: Some(resp) });
            }
            rx = raw;
        }
        Ok(Exchange { tx, rx, response: None })
    }

    /// Navigate the PM5 to the "prepare to row" workout screen.
    pub fn start_workout(&self) -> Result<PrevFrame> {
        self.write(&start_workout_frame())?;
        let raw = self.read()?;
        let resp = csafe::parse_response(&csafe::decode(&raw)?)?;
        Ok(resp.prev_frame())
    }

    fn write(&self, frame: &[u8]) -> Result<()> {
        let mut buf = [0u8; HID_REPORT + 1];
        let n = frame.len().min(HID_REPORT);
        buf[0] = REPORT_ID;
        buf[1..1 + n].copy_from_slice(&frame[..n]);
        self.dev.write(&buf).context("PM5 write")?;
        Ok(())
    }

    /// One HID report, report ID stripped. Empty on timeout.
    fn read(&self) -> Result<Vec<u8>> {
        let mut buf = [0u8; HID_REPORT + 1];
        let n = self
            .dev
            .read_timeout(&mut buf, READ_TIMEOUT_MS)
            .context("PM5 read")?;
        Ok(if n > 1 { buf[1..n].to_vec() } else { Vec::new() })
    }
}

/// Frame that opens the "prepare to row" workout screen.
///
/// Locked to the bytes PM5 firmware accepted over USB in v1:
/// `f1 76 04 13 02 01 01 63 f2`, i.e. SETPMCFG wrapping
/// SET_SCREENSTATE(Workout=1, PrepareToRow=1).
pub fn start_workout_frame() -> Vec<u8> {
    csafe::encode(&[std_cmd::SET_PM_CFG, 0x04, pm_cmd::SET_SCREEN_STATE, 0x02, 0x01, 0x01])
}

pub fn status_cmds() -> Vec<Cmd> {
    vec![
        Cmd::Std(std_cmd::GET_CADENCE),
        Cmd::Std(std_cmd::GET_POWER),
        Cmd::pm(pm_cmd::GET_STROKE_STATE),
        Cmd::pm(pm_cmd::GET_DRAG_FACTOR),
        Cmd::pm(pm_cmd::GET_WORK_DISTANCE),
        Cmd::pm(pm_cmd::GET_WORK_TIME),
        Cmd::pm(pm_cmd::GET_WORKOUT_STATE),
    ]
}

pub fn force_cmds() -> Vec<Cmd> {
    vec![
        Cmd::pm_long(pm_cmd::GET_FORCE_PLOT_DATA, vec![FORCE_BLOCK]),
        Cmd::pm(pm_cmd::GET_STROKE_STATE),
    ]
}

fn stroke_state(r: &Response) -> Option<StrokeState> {
    r.pm(pm_cmd::GET_STROKE_STATE)
        .and_then(|d| d.first())
        .map(|&c| StrokeState::from_code(c))
}

/// Multi-byte values are read little-endian. `rowctl pm5-probe` prints both
/// byte orders so this can be confirmed on hardware.
pub fn decode_status(r: &Response) -> Pm5Status {
    Pm5Status {
        stroke_state: stroke_state(r),
        spm: r.std(std_cmd::GET_CADENCE).and_then(le_u16),
        watts: r.std(std_cmd::GET_POWER).and_then(le_u16),
        distance_dm: r.pm(pm_cmd::GET_WORK_DISTANCE).and_then(le_u32),
        work_time_cs: r.pm(pm_cmd::GET_WORK_TIME).and_then(le_u32),
        drag_factor: r.pm(pm_cmd::GET_DRAG_FACTOR).and_then(|d| d.first().copied()),
        workout_state: r.pm(pm_cmd::GET_WORKOUT_STATE).and_then(|d| d.first().copied()),
    }
}

/// Force reply layout: `bytes_read, data[32]`; points are `bytes_read / 2` u16s.
pub fn decode_force(r: &Response) -> Option<Pm5Force> {
    let d = r.pm(pm_cmd::GET_FORCE_PLOT_DATA)?;
    let (&n, data) = d.split_first()?;
    let (pairs, _) = data[..(n as usize).min(data.len())].as_chunks::<2>();
    let points = pairs.iter().map(|&p| u16::from_le_bytes(p)).collect();
    Some(Pm5Force {
        stroke_state: stroke_state(r),
        points,
    })
}

#[derive(Debug, Clone)]
pub struct Pm5Config {
    /// Time per frame. Frames alternate, so each is polled at half this rate
    /// and the stroke state at the full rate.
    pub interval: Duration,
}

impl Default for Pm5Config {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(25),
        }
    }
}

/// Poll the PM5 until `tx` is closed, reconnecting when it drops (the PM5
/// disconnects from USB when it sleeps).
pub fn run_blocking(clock: Clock, tx: mpsc::Sender<Event>, cfg: Pm5Config) {
    let mut reported_missing = false;
    while !tx.is_closed() {
        match Pm5::open() {
            Ok(pm5) => {
                reported_missing = false;
                info!("PM5 connected: {}", pm5.path);
                if !send_link(&tx, &clock, true, Some(pm5.path.clone())) {
                    return;
                }
                match poll_loop(&pm5, &clock, &tx, &cfg) {
                    Ok(()) => return,
                    Err(e) => {
                        warn!("PM5 dropped: {e:#}");
                        if !send_link(&tx, &clock, false, Some(format!("{e:#}"))) {
                            return;
                        }
                    }
                }
            }
            Err(e) if !reported_missing => {
                reported_missing = true;
                info!("PM5 not available: {e:#}");
                if !send_link(&tx, &clock, false, Some(format!("{e:#}"))) {
                    return;
                }
            }
            Err(_) => {}
        }
        sleep_unless_closed(&tx, Duration::from_secs(2));
    }
}

/// Returns `Ok(())` when the receiver is gone, `Err` when the PM5 is.
fn poll_loop(pm5: &Pm5, clock: &Clock, tx: &mpsc::Sender<Event>, cfg: &Pm5Config) -> Result<()> {
    let frames = [status_cmds(), force_cmds()];
    let mut misses = 0u32;
    loop {
        for (i, cmds) in frames.iter().enumerate() {
            let started = Instant::now();
            let ex = pm5.transact(cmds)?;
            let t_us = clock.now_us();
            match ex.response {
                Some(r) => {
                    misses = 0;
                    let kind = if i == 0 {
                        Some(EventKind::Pm5Status(decode_status(&r)))
                    } else {
                        decode_force(&r)
                            .filter(|f| !f.points.is_empty())
                            .map(EventKind::Pm5Force)
                    };
                    if let Some(kind) = kind
                        && tx.blocking_send(Event { t_us, kind }).is_err()
                    {
                        return Ok(());
                    }
                }
                None => {
                    misses += 1;
                    if misses % 50 == 1 {
                        warn!("PM5: no usable reply ({misses} in a row), last rx {:02x?}", ex.rx);
                    }
                    if misses >= MAX_MISSES {
                        bail!("PM5 stopped answering");
                    }
                }
            }
            if tx.is_closed() {
                return Ok(());
            }
            if let Some(rest) = cfg.interval.checked_sub(started.elapsed()) {
                std::thread::sleep(rest);
            }
        }
    }
}

fn send_link(tx: &mpsc::Sender<Event>, clock: &Clock, connected: bool, detail: Option<String>) -> bool {
    let kind = EventKind::Link(Link {
        device: Device::Pm5,
        connected,
        detail,
    });
    tx.blocking_send(Event { t_us: clock.now_us(), kind }).is_ok()
}

fn sleep_unless_closed(tx: &mpsc::Sender<Event>, total: Duration) {
    let step = Duration::from_millis(100);
    let mut slept = Duration::ZERO;
    while slept < total && !tx.is_closed() {
        std::thread::sleep(step);
        slept += step;
    }
}

/// Commands `rowctl pm5-probe` sends one at a time, to confirm codes and byte
/// order on real hardware.
pub fn probe_cases() -> Vec<(&'static str, Vec<Cmd>)> {
    use Cmd::Std;
    vec![
        ("GETSTATUS (std 0x80)", vec![Std(std_cmd::GET_STATUS)]),
        ("GETVERSION (std 0x91)", vec![Std(std_cmd::GET_VERSION)]),
        ("GETSERIAL (std 0x94)", vec![Std(std_cmd::GET_SERIAL)]),
        ("GETCADENCE (std 0xA7)", vec![Std(std_cmd::GET_CADENCE)]),
        ("GETPOWER (std 0xB4)", vec![Std(std_cmd::GET_POWER)]),
        ("GETPACE (std 0xA6)", vec![Std(std_cmd::GET_PACE)]),
        ("GETCALORIES (std 0xA3)", vec![Std(std_cmd::GET_CALORIES)]),
        ("GETHRCUR (std 0xB0)", vec![Std(std_cmd::GET_HR_CUR)]),
        ("PM STROKESTATE (pm 0xBF)", vec![Cmd::pm(pm_cmd::GET_STROKE_STATE)]),
        ("PM DRAGFACTOR (pm 0xC1)", vec![Cmd::pm(pm_cmd::GET_DRAG_FACTOR)]),
        ("PM WORKDISTANCE (pm 0xA3)", vec![Cmd::pm(pm_cmd::GET_WORK_DISTANCE)]),
        ("PM WORKTIME (pm 0xA0)", vec![Cmd::pm(pm_cmd::GET_WORK_TIME)]),
        ("PM WORKOUTSTATE (pm 0x8D)", vec![Cmd::pm(pm_cmd::GET_WORKOUT_STATE)]),
        ("PM WORKOUTTYPE (pm 0x89)", vec![Cmd::pm(pm_cmd::GET_WORKOUT_TYPE)]),
        (
            "PM FORCEPLOTDATA (pm 0x6B, 32)",
            vec![Cmd::pm_long(pm_cmd::GET_FORCE_PLOT_DATA, vec![FORCE_BLOCK])],
        ),
        ("status frame (as polled)", status_cmds()),
        ("force frame (as polled)", force_cmds()),
    ]
}

/// Run every probe case once.
pub fn probe() -> Result<Vec<(&'static str, Exchange)>> {
    let pm5 = Pm5::open()?;
    let mut out = Vec::new();
    for (label, cmds) in probe_cases() {
        out.push((label, pm5.transact(&cmds)?));
        std::thread::sleep(Duration::from_millis(150));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csafe::{decode, encode, parse_response, payload};

    /// If this changes, the PM5 silently ignores the frame or shows an error.
    #[test]
    fn start_workout_frame_is_accepted_bytes() {
        assert_eq!(
            start_workout_frame(),
            vec![0xF1, 0x76, 0x04, 0x13, 0x02, 0x01, 0x01, 0x63, 0xF2],
        );
    }

    #[test]
    fn status_frame_bytes() {
        assert_eq!(
            payload(&status_cmds()),
            vec![0xA7, 0xB4, 0x1A, 0x05, 0xBF, 0xC1, 0xA3, 0xA0, 0x8D],
        );
    }

    #[test]
    fn force_frame_bytes() {
        assert_eq!(payload(&force_cmds()), vec![0x1A, 0x04, 0x6B, 0x01, 0x20, 0xBF]);
    }

    /// Build a reply payload: status, standard items, then one wrapper.
    fn reply(status: u8, std_items: &[(u8, &[u8])], pm_items: &[(u8, &[u8])]) -> Vec<u8> {
        let mut out = vec![status];
        for (code, data) in std_items {
            out.extend([*code, data.len() as u8]);
            out.extend_from_slice(data);
        }
        let mut wrap = Vec::new();
        for (code, data) in pm_items {
            wrap.extend([*code, data.len() as u8]);
            wrap.extend_from_slice(data);
        }
        out.extend([std_cmd::SET_USER_CFG1, wrap.len() as u8]);
        out.extend(wrap);
        out
    }

    #[test]
    fn decodes_status_reply() {
        let p = reply(
            0x05,
            &[(0xA7, &[20, 0, 0x53]), (0xB4, &[84, 0, 0x58])],
            &[
                (0xBF, &[2]),
                (0xC1, &[112]),
                (0xA3, &[0x10, 0x27, 0, 0, 0]), // 10000 dm
                (0xA0, &[0x70, 0x17, 0, 0, 0]), // 6000 cs
                (0x8D, &[1]),
            ],
        );
        // Through the real framing, not just the parser.
        let r = parse_response(&decode(&encode(&p)).unwrap()).unwrap();
        assert_eq!(
            decode_status(&r),
            Pm5Status {
                stroke_state: Some(StrokeState::Drive),
                spm: Some(20),
                watts: Some(84),
                distance_dm: Some(10_000),
                work_time_cs: Some(6_000),
                drag_factor: Some(112),
                workout_state: Some(1),
            }
        );
    }

    #[test]
    fn decodes_force_reply_using_bytes_read() {
        let mut data = vec![4u8, 100, 0, 200, 0];
        data.resize(33, 0);
        let p = reply(0x05, &[], &[(0x6B, &data), (0xBF, &[2])]);
        let r = parse_response(&p).unwrap();
        let f = decode_force(&r).unwrap();
        assert_eq!(f.points, vec![100, 200]);
        assert_eq!(f.stroke_state, Some(StrokeState::Drive));
    }

    #[test]
    fn missing_items_decode_as_none() {
        let r = parse_response(&[0x05]).unwrap();
        assert_eq!(decode_status(&r), Pm5Status::default());
        assert!(decode_force(&r).is_none());
    }
}
