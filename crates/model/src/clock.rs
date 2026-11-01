use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Wall-clock timestamps that never go backwards within a session.
///
/// The Unix time is read once at start and advanced with a monotonic
/// [`Instant`], so an NTP step mid-session can't reorder events. The browser
/// stamps pose frames the same way (`performance.timeOrigin + performance.now()`),
/// which keeps both streams on one timeline when they run on the same laptop.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    wall0_us: i64,
    inst0: Instant,
}

impl Clock {
    pub fn start() -> Self {
        let wall0_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as i64)
            .unwrap_or(0);
        Self {
            wall0_us,
            inst0: Instant::now(),
        }
    }

    /// Microseconds since the Unix epoch.
    pub fn now_us(&self) -> i64 {
        self.wall0_us + self.inst0.elapsed().as_micros() as i64
    }
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
