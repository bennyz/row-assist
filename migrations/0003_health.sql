-- Daily recovery data from the Apple Watch, posted by an iPhone Shortcut.
-- Upserted by day: a run that was missed is filled in by the next one.
CREATE TABLE health_daily (
    day         TEXT PRIMARY KEY,   -- YYYY-MM-DD, local; sleep counts for the morning it ended
    hrv_ms      REAL,               -- mean SDNN of the day's samples
    resting_hr  REAL,
    sleep_h     REAL,               -- time asleep (stages merged, sources de-overlapped)
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
