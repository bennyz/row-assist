//! Spoken cues through the OS text-to-speech command.
//!
//! A worker thread speaks one line at a time. The queue holds two lines and
//! drops anything beyond that, so a backlog never builds up mid-row.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{SyncSender, sync_channel};

#[cfg(target_os = "macos")]
const PROGRAMS: &[&str] = &["say"];
#[cfg(not(target_os = "macos"))]
const PROGRAMS: &[&str] = &["spd-say", "espeak-ng", "espeak"];

#[derive(Clone)]
pub struct Speaker {
    tx: SyncSender<String>,
    pub program: String,
}

impl Speaker {
    /// `None` when no speech program is installed.
    pub fn start() -> Option<Self> {
        let path = find_program()?;
        let program = path.file_name()?.to_string_lossy().into_owned();
        // spd-say returns at once; -w makes it wait so lines don't overlap.
        let wait = program == "spd-say";
        let (tx, rx) = sync_channel::<String>(2);
        std::thread::spawn(move || {
            for text in rx {
                let mut cmd = Command::new(&path);
                if wait {
                    cmd.arg("-w");
                }
                let _ = cmd
                    .arg(text)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        });
        Some(Self { tx, program })
    }

    pub fn say(&self, text: &str) {
        let _ = self.tx.try_send(text.to_owned());
    }

    pub fn candidates() -> &'static [&'static str] {
        PROGRAMS
    }
}

fn find_program() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    PROGRAMS
        .iter()
        .find_map(|name| dirs.iter().map(|d| d.join(name)).find(|p| p.is_file()))
}
