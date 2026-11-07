use std::time::{SystemTime, UNIX_EPOCH};

/// Wall-clock timestamps that never go backwards.
///
/// The Unix time is read once at start and advanced with a monotonic clock,
/// so an NTP step mid-session can't reorder events. On Linux that clock is
/// `CLOCK_BOOTTIME`, which keeps counting while the laptop sleeps: the server
/// runs for days, and a clock that paused overnight would date the next
/// morning's row hours early. The browser stamps pose frames on its own
/// clock and converts them to this one (`ClockSync` in `pose.js`).
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    wall0_us: i64,
    mono0_us: i64,
}

impl Clock {
    pub fn start() -> Self {
        let wall0_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as i64)
            .unwrap_or(0);
        Self {
            wall0_us,
            mono0_us: mono_us(),
        }
    }

    /// Microseconds since the Unix epoch.
    pub fn now_us(&self) -> i64 {
        self.wall0_us + (mono_us() - self.mono0_us)
    }
}

#[cfg(target_os = "linux")]
// `time_t` and `c_long` are only `i64` on 64-bit targets.
#[allow(clippy::useless_conversion)]
fn mono_us() -> i64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid timespec to write into; CLOCK_BOOTTIME exists
    // on every Linux this builds for.
    unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) };
    i64::from(ts.tv_sec) * 1_000_000 + i64::from(ts.tv_nsec) / 1_000
}

#[cfg(not(target_os = "linux"))]
fn mono_us() -> i64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_micros() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_goes_backwards() {
        let clock = Clock::start();
        let mut prev = clock.now_us();
        for _ in 0..1000 {
            let now = clock.now_us();
            assert!(now >= prev);
            prev = now;
        }
    }

    #[test]
    fn is_close_to_system_time() {
        let clock = Clock::start();
        let sys = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as i64;
        assert!((clock.now_us() - sys).abs() < 1_000_000);
    }
}
