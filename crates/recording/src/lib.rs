//! Session recordings: every event a session produced, in order, on disk.
//!
//! Layout: the 8-byte magic `ROWREC01`, then one zstd stream of frames. Each
//! frame is a little-endian `u32` length followed by a MessagePack value with
//! structs encoded as maps, so adding an optional field later doesn't break
//! old files. The first frame is the [`Header`]; every later frame is a
//! [`model::Event`].
//!
//! The writer flushes a zstd block every second, so a crash loses at most the
//! last second, and the reader stops cleanly at a truncated tail.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use model::Event;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const MAGIC: &[u8; 8] = b"ROWREC01";
pub const FORMAT_VERSION: u32 = 1;
const MAX_FRAME: usize = 16 << 20;
const FLUSH_EVERY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    pub format: u32,
    /// Session start, microseconds since the Unix epoch.
    pub started_at_us: i64,
    /// Program and version that wrote the file.
    pub app: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub simulated: bool,
    /// Workout plan the session was rowed to, e.g. `reference`.
    #[serde(default)]
    pub plan: Option<String>,
    /// The technique fault the coach was set to cue, e.g. `early arms`.
    #[serde(default)]
    pub focus: Option<String>,
}

impl Header {
    pub fn new(started_at_us: i64, app: impl Into<String>) -> Self {
        Self {
            format: FORMAT_VERSION,
            started_at_us,
            app: app.into(),
            note: None,
            simulated: false,
            plan: None,
            focus: None,
        }
    }
}

pub type FileWriter = Writer<BufWriter<File>>;

/// Where the camera stills taken during a row go: next to its recording,
/// `20261107-091500.rowrec` → `20261107-091500.stills/`, one JPEG per still
/// named by its session time (µs, the same clock as the events).
pub fn stills_dir(recording: &Path) -> std::path::PathBuf {
    recording.with_extension("stills")
}

pub struct Writer<W: Write> {
    enc: zstd::Encoder<'static, W>,
    last_flush: Instant,
    events: u64,
}

impl FileWriter {
    pub fn create(path: &Path, header: &Header) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        }
        let file = File::create(path).with_context(|| format!("create {}", path.display()))?;
        Self::new(BufWriter::new(file), header)
    }
}

impl<W: Write> Writer<W> {
    pub fn new(mut inner: W, header: &Header) -> Result<Self> {
        inner.write_all(MAGIC)?;
        let mut w = Self {
            enc: zstd::Encoder::new(inner, 3)?,
            last_flush: Instant::now(),
            events: 0,
        };
        w.frame(header)?;
        w.flush()?;
        Ok(w)
    }

    pub fn write(&mut self, ev: &Event) -> Result<()> {
        self.frame(ev)?;
        self.events += 1;
        if self.last_flush.elapsed() >= FLUSH_EVERY {
            self.flush()?;
        }
        Ok(())
    }

    /// End the current zstd block so everything so far survives a crash.
    pub fn flush(&mut self) -> Result<()> {
        self.enc.flush()?;
        self.last_flush = Instant::now();
        Ok(())
    }

    pub fn events(&self) -> u64 {
        self.events
    }

    pub fn get_ref(&self) -> &W {
        self.enc.get_ref()
    }

    /// Close the zstd stream. Without this the file still reads back, but is
    /// reported as truncated.
    pub fn finish(self) -> Result<W> {
        let mut inner = self.enc.finish()?;
        inner.flush()?;
        Ok(inner)
    }

    fn frame<T: Serialize>(&mut self, v: &T) -> Result<()> {
        let bytes = rmp_serde::to_vec_named(v)?;
        self.enc.write_all(&(bytes.len() as u32).to_le_bytes())?;
        self.enc.write_all(&bytes)?;
        Ok(())
    }
}

pub struct Reader<R: Read> {
    dec: zstd::Decoder<'static, BufReader<R>>,
    header: Header,
    truncated: bool,
    done: bool,
}

impl Reader<File> {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        Self::new(file).with_context(|| format!("read {}", path.display()))
    }
}

impl<R: Read> Reader<R> {
    pub fn new(mut inner: R) -> Result<Self> {
        let mut magic = [0u8; 8];
        inner.read_exact(&mut magic).context("missing magic")?;
        if &magic != MAGIC {
            bail!("not a Row Assist recording");
        }
        let mut dec = zstd::Decoder::new(inner)?;
        let header: Header = match read_frame(&mut dec) {
            Ok(Some(h)) => h,
            Ok(None) | Err(FrameError::Truncated) => bail!("recording has no header"),
            Err(FrameError::Other(e)) => return Err(e),
        };
        if header.format > FORMAT_VERSION {
            bail!("recording format {} is newer than this build ({FORMAT_VERSION})", header.format);
        }
        Ok(Self {
            dec,
            header,
            truncated: false,
            done: false,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// True if the file ended mid-stream (the writer never finished).
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

impl<R: Read> Iterator for Reader<R> {
    type Item = Result<Event>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match read_frame(&mut self.dec) {
            Ok(Some(ev)) => Some(Ok(ev)),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(FrameError::Truncated) => {
                self.done = true;
                self.truncated = true;
                None
            }
            Err(FrameError::Other(e)) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

/// Read a whole recording: header, events, and whether the tail was cut off.
pub fn read_all(path: &Path) -> Result<(Header, Vec<Event>, bool)> {
    let mut reader = Reader::open(path)?;
    let events = reader.by_ref().collect::<Result<Vec<_>>>()?;
    Ok((reader.header.clone(), events, reader.truncated))
}

enum FrameError {
    Truncated,
    Other(anyhow::Error),
}

impl From<io::Error> for FrameError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            Self::Truncated
        } else {
            Self::Other(e.into())
        }
    }
}

/// `Ok(None)` at a clean end of stream.
fn read_frame<T: DeserializeOwned>(r: &mut impl Read) -> Result<Option<T>, FrameError> {
    let mut len = [0u8; 4];
    let n = read_full(r, &mut len)?;
    if n == 0 {
        return Ok(None);
    }
    if n < len.len() {
        return Err(FrameError::Truncated);
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(FrameError::Other(anyhow::anyhow!("frame of {len} bytes is too large")));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    rmp_serde::from_slice(&buf)
        .map(Some)
        .map_err(|e| FrameError::Other(e.into()))
}

/// Like `read_exact`, but returns how many bytes it got before a clean EOF.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(got)
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{EventKind, Heart, Pm5Force, Pm5Status, StrokeState};

    fn sample_events() -> Vec<Event> {
        vec![
            Event {
                t_us: 1,
                kind: EventKind::Pm5Status(Pm5Status {
                    stroke_state: Some(StrokeState::Drive),
                    spm: Some(20),
                    watts: Some(84),
                    ..Default::default()
                }),
            },
            Event {
                t_us: 2,
                kind: EventKind::Pm5Force(Pm5Force {
                    stroke_state: Some(StrokeState::Unknown(9)),
                    points: vec![10, 20, 30],
                }),
            },
            Event {
                t_us: 3,
                kind: EventKind::Heart(Heart {
                    bpm: 140,
                    rr_1024: vec![439],
                    contact: Some(true),
                    ..Default::default()
                }),
            },
            Event {
                t_us: 4,
                kind: EventKind::Marker { label: "early arms".into() },
            },
        ]
    }

    #[test]
    fn round_trips_header_and_events() {
        let header = Header::new(42, "test");
        let mut w = Writer::new(Vec::new(), &header).unwrap();
        for ev in sample_events() {
            w.write(&ev).unwrap();
        }
        assert_eq!(w.events(), 4);
        let bytes = w.finish().unwrap();

        let mut r = Reader::new(bytes.as_slice()).unwrap();
        assert_eq!(r.header(), &header);
        let events: Vec<_> = r.by_ref().collect::<Result<_>>().unwrap();
        assert_eq!(events, sample_events());
        assert!(!r.truncated());
    }

    #[test]
    fn unfinished_file_reads_up_to_last_flush() {
        let mut w = Writer::new(Vec::new(), &Header::new(0, "test")).unwrap();
        let events = sample_events();
        w.write(&events[0]).unwrap();
        w.write(&events[1]).unwrap();
        w.flush().unwrap();
        // Simulate a crash: take the bytes without finishing the stream.
        let bytes = w.get_ref().clone();

        let mut r = Reader::new(bytes.as_slice()).unwrap();
        let got: Vec<_> = r.by_ref().collect::<Result<_>>().unwrap();
        assert_eq!(got, events[..2]);
        assert!(r.truncated());
    }

    #[test]
    fn rejects_other_files() {
        assert!(Reader::new(&b"not a recording"[..]).is_err());
    }

    #[test]
    fn rejects_newer_formats() {
        let mut header = Header::new(0, "future");
        header.format = FORMAT_VERSION + 1;
        let bytes = Writer::new(Vec::new(), &header).unwrap().finish().unwrap();
        assert!(Reader::new(bytes.as_slice()).is_err());
    }

    #[test]
    fn old_headers_without_optional_fields_still_read() {
        #[derive(Serialize)]
        struct OldHeader {
            format: u32,
            started_at_us: i64,
            app: String,
        }
        let old = OldHeader {
            format: 1,
            started_at_us: 7,
            app: "v0".into(),
        };
        let mut bytes = MAGIC.to_vec();
        let mut enc = zstd::Encoder::new(&mut bytes, 3).unwrap();
        let frame = rmp_serde::to_vec_named(&old).unwrap();
        enc.write_all(&(frame.len() as u32).to_le_bytes()).unwrap();
        enc.write_all(&frame).unwrap();
        enc.finish().unwrap();

        let r = Reader::new(bytes.as_slice()).unwrap();
        assert_eq!(r.header().note, None);
        assert!(!r.header().simulated);
        assert_eq!(r.header().plan, None);
    }
}
