-- Workouts imported from the Concept2 online logbook (CSV export or API).
CREATE TABLE concept2_results (
    log_id          INTEGER PRIMARY KEY,   -- Concept2 "Log ID"
    started_at      TEXT    NOT NULL,      -- local time as exported: YYYY-MM-DD HH:MM:SS
    description     TEXT    NOT NULL,
    work_time_s     REAL    NOT NULL,
    rest_time_s     REAL,
    work_distance_m INTEGER NOT NULL,
    rest_distance_m INTEGER,
    stroke_rate     REAL,
    stroke_count    INTEGER,               -- includes strokes during rest intervals
    pace_s_per_500  REAL,
    avg_watts       REAL,
    cal_per_hour    REAL,
    total_cal       REAL,
    avg_hr          INTEGER,               -- NULL when no monitor (the CSV says 0)
    drag_factor     INTEGER,
    age             INTEGER,
    weight_class    TEXT,
    machine         TEXT    NOT NULL,      -- RowErg, SkiErg, BikeErg
    ranked          INTEGER NOT NULL,
    comments        TEXT,
    entered_at      TEXT,
    imported_at     TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX concept2_results_started_at ON concept2_results (started_at);
