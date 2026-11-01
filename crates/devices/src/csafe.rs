//! CSAFE framing and command encoding, as spoken by the Concept2 PM5.
//!
//! A standard frame is `F1 <payload> <xor checksum> F2`. Any payload or
//! checksum byte in `F0..=F3` is escaped as `F3, b - 0xF0` (so `00..=03`),
//! which keeps flag bytes out of the frame body. Concept2-specific
//! ("PM proprietary") commands travel inside a `SETUSERCFG1` (0x1A) wrapper, so
//! the same byte can mean different things: bare `0xA3` is the standard
//! GETCALORIES, while `0xA3` inside the wrapper is the PM's work distance.
//!
//! Framing and checksums are ported from v1, where they were checked against
//! PM5 hardware. v1 escaped with `b ^ 0x03`, which turns `F1` into `F3 F2` (a
//! STOP flag) and garbles any reply byte in `F0..=F3`; that's fixed here.
//! Command codes follow the published CSAFE / PM tables
//! (the ones pyrow and ErgometerJS use); `rowctl pm5-probe` confirms them on a
//! real erg.

use thiserror::Error;

pub const START: u8 = 0xF1;
pub const STOP: u8 = 0xF2;
pub const STUFF: u8 = 0xF3;

/// Standard CSAFE commands.
pub mod std_cmd {
    pub const GET_STATUS: u8 = 0x80;
    pub const GET_VERSION: u8 = 0x91;
    pub const GET_SERIAL: u8 = 0x94;
    pub const GET_CALORIES: u8 = 0xA3;
    pub const GET_PACE: u8 = 0xA6;
    pub const GET_CADENCE: u8 = 0xA7;
    pub const GET_HR_CUR: u8 = 0xB0;
    pub const GET_POWER: u8 = 0xB4;
    /// Wrapper carrying Concept2 proprietary commands.
    pub const SET_USER_CFG1: u8 = 0x1A;
    /// Wrapper carrying PM configuration commands. v1 found that only
    /// SET_SCREENSTATE is accepted over USB; workout setup gets PREVREJECT.
    pub const SET_PM_CFG: u8 = 0x76;
}

/// Concept2 proprietary commands, sent inside [`std_cmd::SET_USER_CFG1`].
pub mod pm_cmd {
    pub const GET_WORKOUT_TYPE: u8 = 0x89;
    pub const GET_WORKOUT_STATE: u8 = 0x8D;
    pub const GET_WORK_TIME: u8 = 0xA0;
    pub const GET_WORK_DISTANCE: u8 = 0xA3;
    pub const GET_STROKE_STATE: u8 = 0xBF;
    pub const GET_DRAG_FACTOR: u8 = 0xC1;
    /// Long command. Argument: bytes to read (max 32 = 16 points). Reading
    /// drains the PM's buffer for the current stroke.
    pub const GET_FORCE_PLOT_DATA: u8 = 0x6B;
    /// SET_PM_CFG sub-command (confirmed on hardware in v1).
    pub const SET_SCREEN_STATE: u8 = 0x13;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CsafeError {
    #[error("no start flag")]
    NoStart,
    #[error("no stop flag")]
    NoStop,
    #[error("empty frame")]
    Empty,
    #[error("checksum mismatch: got {got:#04x}, expected {expected:#04x}")]
    Checksum { got: u8, expected: u8 },
    #[error("response truncated at byte {0}")]
    Truncated(usize),
    #[error("invalid escape {0:#04x} after stuff byte")]
    BadStuff(u8),
}

/// One command to put in a request frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    /// Standard short command (`0x80..=0xFF`, no arguments).
    Std(u8),
    /// Concept2 proprietary command. Codes below `0x80` are long commands and
    /// always carry a length byte plus `args`.
    Pm { code: u8, args: Vec<u8> },
}

impl Cmd {
    pub fn pm(code: u8) -> Self {
        Self::Pm { code, args: Vec::new() }
    }

    pub fn pm_long(code: u8, args: Vec<u8>) -> Self {
        Self::Pm { code, args }
    }

    /// `(code, is_pm)`: what the matching response item looks like.
    pub fn key(&self) -> (u8, bool) {
        match self {
            Self::Std(code) => (*code, false),
            Self::Pm { code, .. } => (*code, true),
        }
    }
}

/// Build an unframed request payload. Consecutive PM commands share one
/// `SETUSERCFG1` wrapper.
pub fn payload(cmds: &[Cmd]) -> Vec<u8> {
    fn flush(out: &mut Vec<u8>, wrap: &mut Vec<u8>) {
        if !wrap.is_empty() {
            out.push(std_cmd::SET_USER_CFG1);
            out.push(wrap.len() as u8);
            out.append(wrap);
        }
    }

    let mut out = Vec::new();
    let mut wrap = Vec::new();
    for cmd in cmds {
        match cmd {
            Cmd::Std(code) => {
                flush(&mut out, &mut wrap);
                out.push(*code);
            }
            Cmd::Pm { code, args } => {
                wrap.push(*code);
                if *code < 0x80 {
                    wrap.push(args.len() as u8);
                    wrap.extend_from_slice(args);
                }
            }
        }
    }
    flush(&mut out, &mut wrap);
    out
}

/// Wrap a payload in a CSAFE frame with checksum and byte-stuffing.
pub fn encode(payload: &[u8]) -> Vec<u8> {
    let cksum = payload.iter().fold(0u8, |a, &b| a ^ b);
    let mut out = vec![START];
    for &b in payload.iter().chain(std::iter::once(&cksum)) {
        if matches!(b, 0xF0..=0xF3) {
            out.push(STUFF);
            out.push(b - 0xF0);
        } else {
            out.push(b);
        }
    }
    out.push(STOP);
    out
}

/// Unframe, unstuff and verify a frame. Returns the payload without the
/// checksum. Anything before the start flag (e.g. a HID report ID) is skipped.
pub fn decode(raw: &[u8]) -> Result<Vec<u8>, CsafeError> {
    let s = raw
        .iter()
        .position(|&b| b == START)
        .ok_or(CsafeError::NoStart)?;
    // The first STOP after START: the PM5 sometimes sends a double STOP, and
    // escaped bytes are 00..=03, so a STOP inside the body can't occur.
    let e = raw[s + 1..]
        .iter()
        .position(|&b| b == STOP)
        .map(|p| p + s + 1)
        .ok_or(CsafeError::NoStop)?;

    let mut out = Vec::with_capacity(e - s);
    let mut i = s + 1;
    while i < e {
        if raw[i] == STUFF {
            i += 1;
            match raw.get(i) {
                Some(&n) if i < e && n <= 0x03 => out.push(0xF0 + n),
                Some(&n) if i < e => return Err(CsafeError::BadStuff(n)),
                _ => return Err(CsafeError::BadStuff(STOP)),
            }
        } else {
            out.push(raw[i]);
        }
        i += 1;
    }

    let got = out.pop().ok_or(CsafeError::Empty)?;
    let expected = out.iter().fold(0u8, |a, &b| a ^ b);
    if got != expected {
        return Err(CsafeError::Checksum { got, expected });
    }
    Ok(out)
}

/// What the PM thought of the *previous* frame (status bits 4-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrevFrame {
    Ok,
    Rejected,
    Bad,
    NotReady,
}

/// A parsed response: the status byte plus one item per answered command.
/// Items from inside the `SETUSERCFG1` wrapper are flattened with `pm = true`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u8,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub code: u8,
    pub pm: bool,
    pub data: Vec<u8>,
}

impl Item {
    pub fn key(&self) -> (u8, bool) {
        (self.code, self.pm)
    }
}

impl Response {
    pub fn std(&self, code: u8) -> Option<&[u8]> {
        self.find(code, false)
    }

    pub fn pm(&self, code: u8) -> Option<&[u8]> {
        self.find(code, true)
    }

    fn find(&self, code: u8, pm: bool) -> Option<&[u8]> {
        self.items
            .iter()
            .find(|i| i.code == code && i.pm == pm)
            .map(|i| i.data.as_slice())
    }

    pub fn prev_frame(&self) -> PrevFrame {
        match self.status & 0x30 {
            0x00 => PrevFrame::Ok,
            0x10 => PrevFrame::Rejected,
            0x20 => PrevFrame::Bad,
            _ => PrevFrame::NotReady,
        }
    }

    /// Slave state machine (status bits 0-3): 1 ready, 2 idle, 5 in use, 7 finished...
    pub fn machine_state(&self) -> u8 {
        self.status & 0x0F
    }
}

/// Parse a decoded payload: `status, (code, len, data[len])*`, recursing
/// once into a `SETUSERCFG1` item.
pub fn parse_response(payload: &[u8]) -> Result<Response, CsafeError> {
    let (&status, rest) = payload.split_first().ok_or(CsafeError::Empty)?;
    let mut items = Vec::new();
    parse_items(rest, false, 1, &mut items)?;
    Ok(Response { status, items })
}

fn parse_items(b: &[u8], in_wrapper: bool, base: usize, out: &mut Vec<Item>) -> Result<(), CsafeError> {
    let mut i = 0;
    while i < b.len() {
        let code = b[i];
        let len = *b.get(i + 1).ok_or(CsafeError::Truncated(base + i))? as usize;
        let data = b
            .get(i + 2..i + 2 + len)
            .ok_or(CsafeError::Truncated(base + i))?;
        if !in_wrapper && code == std_cmd::SET_USER_CFG1 {
            parse_items(data, true, base + i + 2, out)?;
        } else {
            out.push(Item {
                code,
                pm: in_wrapper,
                data: data.to_vec(),
            });
        }
        i += 2 + len;
    }
    Ok(())
}

pub fn le_u16(b: &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes([*b.first()?, *b.get(1)?]))
}

pub fn le_u32(b: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_computes_xor_checksum() {
        assert_eq!(encode(&[0x94]), vec![0xF1, 0x94, 0x94, 0xF2]);
        // 0x76 ^ 0x04 ^ 0x13 ^ 0x02 ^ 0x01 ^ 0x01 = 0x63.
        let frame = encode(&[0x76, 0x04, 0x13, 0x02, 0x01, 0x01]);
        assert_eq!(frame[frame.len() - 2], 0x63);
    }

    #[test]
    fn encode_byte_stuffs_reserved_bytes() {
        // [START, STUFF, 0xF0-0xF0, STUFF, (checksum 0xF0)-0xF0, STOP]
        assert_eq!(encode(&[0xF0]), vec![0xF1, 0xF3, 0x00, 0xF3, 0x00, 0xF2]);
        assert_eq!(encode(&[0xF3, 0x00]), vec![0xF1, 0xF3, 0x03, 0x00, 0xF3, 0x03, 0xF2]);
    }

    #[test]
    fn stuffed_frames_never_contain_flags_in_the_body() {
        for b in 0..=255u8 {
            let frame = encode(&[b, 0x10]);
            let body = &frame[1..frame.len() - 1];
            assert!(!body.contains(&START) && !body.contains(&STOP), "{b:#04x}: {frame:02x?}");
        }
    }

    #[test]
    fn decode_unstuffs_and_verifies_checksum() {
        // 0xF1 is the case v1's XOR escape turned into a STOP flag.
        let payload = [0x76, 0x04, 0x13, 0xF2, 0x01, 0xF1, 0xF0, 0xF3];
        assert_eq!(decode(&encode(&payload)).unwrap(), payload);
    }

    #[test]
    fn decode_round_trips_every_byte_value() {
        let payload: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&payload)).unwrap(), payload);
    }

    #[test]
    fn decode_rejects_bad_escape() {
        assert_eq!(decode(&[0xF1, 0xF3, 0x07, 0xF2]), Err(CsafeError::BadStuff(0x07)));
    }

    #[test]
    fn decode_skips_leading_bytes_and_double_stop() {
        let mut raw = vec![0x02];
        raw.extend(encode(&[0x81, 0xB4, 0x01, 0x10]));
        raw.push(STOP);
        raw.extend([0, 0, 0]);
        assert_eq!(decode(&raw).unwrap(), vec![0x81, 0xB4, 0x01, 0x10]);
    }

    #[test]
    fn decode_rejects_bad_checksum() {
        let mut frame = encode(&[0x94]);
        let i = frame.len() - 2;
        frame[i] ^= 0xFF;
        assert!(matches!(decode(&frame), Err(CsafeError::Checksum { .. })));
    }

    #[test]
    fn payload_groups_pm_commands_in_one_wrapper() {
        let p = payload(&[
            Cmd::Std(std_cmd::GET_CADENCE),
            Cmd::Std(std_cmd::GET_POWER),
            Cmd::pm(pm_cmd::GET_STROKE_STATE),
            Cmd::pm(pm_cmd::GET_DRAG_FACTOR),
        ]);
        assert_eq!(p, vec![0xA7, 0xB4, 0x1A, 0x02, 0xBF, 0xC1]);
    }

    #[test]
    fn payload_gives_long_pm_commands_a_length() {
        let p = payload(&[
            Cmd::pm_long(pm_cmd::GET_FORCE_PLOT_DATA, vec![32]),
            Cmd::pm(pm_cmd::GET_STROKE_STATE),
        ]);
        assert_eq!(p, vec![0x1A, 0x04, 0x6B, 0x01, 0x20, 0xBF]);
    }

    #[test]
    fn parse_flattens_wrapper_items() {
        let payload = [
            0x05, // status: prev OK, in use
            0xB4, 0x03, 0x54, 0x00, 0x58, // power 84 W, unit
            0x1A, 0x06, // wrapper
            0xBF, 0x01, 0x02, // stroke state: drive
            0xC1, 0x01, 0x70, // drag factor 112
        ];
        let r = parse_response(&payload).unwrap();
        assert_eq!(r.prev_frame(), PrevFrame::Ok);
        assert_eq!(r.machine_state(), 5);
        assert_eq!(r.std(std_cmd::GET_POWER), Some(&[0x54, 0x00, 0x58][..]));
        assert_eq!(r.pm(pm_cmd::GET_STROKE_STATE), Some(&[0x02][..]));
        assert_eq!(r.pm(pm_cmd::GET_DRAG_FACTOR), Some(&[0x70][..]));
        // Same code, different space: bare 0xBF is not the PM stroke state.
        assert_eq!(r.std(pm_cmd::GET_STROKE_STATE), None);
    }

    #[test]
    fn parse_reports_truncation() {
        assert_eq!(
            parse_response(&[0x01, 0xB4, 0x03, 0x54]),
            Err(CsafeError::Truncated(1))
        );
    }

    #[test]
    fn status_only_reply_parses_with_no_items() {
        let r = parse_response(&[0x11]).unwrap();
        assert!(r.items.is_empty());
        assert_eq!(r.prev_frame(), PrevFrame::Rejected);
    }

    #[test]
    fn prev_frame_ignores_toggle_and_state_bits() {
        // PREVOK with the frame-toggle bit (0x80) and various states set.
        for status in [0x01u8, 0x07, 0x81, 0x87] {
            let r = Response { status, items: vec![] };
            assert_eq!(r.prev_frame(), PrevFrame::Ok, "{status:#04x}");
        }
        for (status, want) in [(0x91u8, PrevFrame::Rejected), (0x21, PrevFrame::Bad), (0x31, PrevFrame::NotReady)] {
            let r = Response { status, items: vec![] };
            assert_eq!(r.prev_frame(), want, "{status:#04x}");
        }
    }
}
