-- Sessions recorded by the app. Derived from the .rowrec files, which stay the
-- source of truth: re-analysing a recording replaces its row and its strokes.
CREATE TABLE sessions (
    id               INTEGER PRIMARY KEY,
    started_at_us    INTEGER NOT NULL UNIQUE,  -- recording header: identifies the recording
    recording        TEXT    NOT NULL,         -- path of the .rowrec when analysed
    note             TEXT,
    plan             TEXT,
    simulated        INTEGER NOT NULL,
    analysis_version INTEGER NOT NULL,
    seconds          REAL    NOT NULL,         -- rowing time (catch to catch, pauses excluded)
    strokes          INTEGER NOT NULL,
    distance_m       REAL,
    watts            REAL,
    spm              REAL,
    hr               REAL,
    ef               REAL,
    decoupling_pct   REAL,
    watts_at_20      REAL,
    drag_factor      INTEGER,
    peak_hr          REAL,                     -- highest 10 s mean
    trimp            REAL,
    summary          TEXT    NOT NULL,         -- analysis::session::Summary as JSON
    analyzed_at      TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE strokes (
    session_id  INTEGER NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    idx         INTEGER NOT NULL,
    t_catch_us  INTEGER NOT NULL,
    drive_ms    REAL,
    recovery_ms REAL,
    watts       INTEGER,
    spm         INTEGER,
    distance_m  REAL,
    drag_factor INTEGER,
    hr          REAL,
    peak_n      REAL,
    record      TEXT    NOT NULL,              -- analysis::strokes::StrokeRecord as JSON
    PRIMARY KEY (session_id, idx)
) WITHOUT ROWID;

-- The rower's settings. One row.
CREATE TABLE profile (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    hr_rest     INTEGER,
    hr_max      INTEGER,
    sex         TEXT CHECK (sex IN ('male', 'female')),
    drag_target INTEGER,
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
