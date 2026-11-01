//! rowctl: record, replay and inspect Row Assist sessions from the terminal.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod ctx;
mod health;
mod history;
mod inspect;
mod live;
mod plan;
mod probe;
mod profile;
mod render;
mod review;
mod simulate;
mod speak;

#[derive(Parser)]
#[command(name = "rowctl", version, about = "Record, replay and inspect Row Assist sessions")]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Connect to the PM5 and H10 and record a session (Ctrl-C to stop),
    /// then analyse and save it
    Record {
        #[command(flatten)]
        live: live::LiveArgs,
        /// Output file [default: <data>/sessions/<timestamp>.rowrec]
        #[arg(long)]
        out: Option<PathBuf>,
        /// Free-text note stored in the recording header
        #[arg(long)]
        note: Option<String>,
    },
    /// Connect and show live data without recording
    Watch {
        #[command(flatten)]
        live: live::LiveArgs,
    },
    /// Print a recording as JSON lines (header first)
    Dump { file: PathBuf },
    /// Print a recording's events with their original timing
    Replay {
        file: PathBuf,
        /// Playback speed multiplier
        #[arg(long, default_value_t = 1.0)]
        speed: f64,
    },
    /// Show every stroke's force curve and timing in a recording
    Curves { file: PathBuf },
    /// Analyse recordings and save them as sessions
    Analyze {
        files: Vec<PathBuf>,
        /// Every recording in <data>/sessions (e.g. after an analysis update)
        #[arg(long, conflicts_with = "files")]
        all: bool,
        /// Print only, don't save
        #[arg(long)]
        no_save: bool,
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// List saved sessions
    Sessions {
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Show or set heart-rate profile and drag factor target
    Profile(profile::ProfileArgs),
    /// Apple Watch recovery data and today's readiness
    Health(health::HealthArgs),
    /// Serve the review screen in the browser
    Serve {
        /// Address to listen on (0.0.0.0:8080 to reach it from another machine)
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: std::net::SocketAddr,
        #[arg(long)]
        db: Option<PathBuf>,
        /// Built frontend [default: frontend/dist in this checkout]
        #[arg(long, env = "ROW_ASSIST_WEB")]
        web: Option<PathBuf>,
    },
    /// Write a simulated session to a recording, instantly (for testing)
    Simulate {
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 20.0)]
        minutes: f64,
        #[arg(long, default_value_t = 53.0)]
        watts: f64,
        #[arg(long, default_value_t = 20.0)]
        spm: f64,
        /// Heart rate at the start and after 5 min, e.g. 120-142
        #[arg(long, default_value = "120-142", value_parser = plan::parse_bpm)]
        hr: (f64, f64),
        #[arg(long, value_enum)]
        plan: Option<plan::Plan>,
        #[arg(long)]
        note: Option<String>,
        /// Also simulate the camera page's pose landmarks
        #[arg(long)]
        pose: bool,
        /// Make the simulated rower row with a technique fault (implies --pose)
        #[arg(long, value_enum)]
        fault: Option<simulate::FaultArg>,
    },
    /// Send each CSAFE command to the PM5 on its own and show the raw reply
    Pm5Probe,
    /// Import a Concept2 logbook CSV export
    ImportC2 {
        file: PathBuf,
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Show imported Concept2 history with per-stroke metrics
    History {
        #[arg(long)]
        db: Option<PathBuf>,
    },
}

#[cfg(unix)]
fn default_sigpipe() {
    // SAFETY: called before any threads that write to pipes; SIG_DFL is a valid handler.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn default_sigpipe() {}

/// The built frontend in the checkout this binary was built from.
pub fn default_web_dir() -> PathBuf {
    std::env::var_os("ROW_ASSIST_WEB")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../frontend/dist")))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn,devices=info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    // Commands that print and exit should stop quietly when their output is
    // piped into something that closes early (`rowctl health | head`), as
    // Unix tools do. Not the ones that own devices and sockets.
    if !matches!(cli.cmd, Command::Record { .. } | Command::Watch { .. } | Command::Serve { .. }) {
        default_sigpipe();
    }
    let result = match cli.cmd {
        Command::Record { live, out, note } => live::record(live, out, note).await,
        Command::Watch { live } => live::watch(live).await,
        Command::Dump { file } => inspect::dump(&file),
        Command::Replay { file, speed } => inspect::replay(&file, speed).await,
        Command::Curves { file } => inspect::curves(&file),
        Command::Analyze { files, all, no_save, db } => review::analyze(files, all, no_save, db).await,
        Command::Sessions { db } => review::sessions(db).await,
        Command::Profile(args) => profile::run(args).await,
        Command::Health(args) => health::run(args).await,
        Command::Serve { addr, db, web } => {
            server::serve(server::Config {
                addr,
                db: db.unwrap_or_else(store::paths::db_path),
                web_dir: web.unwrap_or_else(default_web_dir),
                token: Some(store::health::token(&store::paths::token_path())?),
            })
            .await
        }
        Command::Simulate { out, minutes, watts, spm, hr, plan, note, pose, fault } => simulate::run(
            &out,
            simulate::SimArgs { minutes, watts, spm, hr, plan, note, pose, fault },
        ),
        Command::Pm5Probe => probe::run().await,
        Command::ImportC2 { file, db } => history::import(&file, db).await,
        Command::History { db } => history::show(db).await,
    };
    // `rowctl dump x | head` closing the pipe early is not an error.
    match result {
        Err(e) if e.downcast_ref::<std::io::Error>().map(|e| e.kind()) == Some(std::io::ErrorKind::BrokenPipe) => Ok(()),
        other => other,
    }
}
