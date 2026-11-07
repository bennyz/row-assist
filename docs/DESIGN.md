# Row Assist v2 — Design

Status: draft, 2026-11-01.

Row Assist helps one rower improve on a Concept2 RowErg by measuring three things
together, every stroke: **effort** (heart rate, power), **technique** (force curve,
body posture from a webcam) and **recovery** (Apple Watch overnight data). It then
closes the loop with a plan: what to row next, and the one technique point to work on.

---

## 1. Goals and starting point

Goals, all four: clean technique, aerobic base, a race-time PR, general fitness.

The rower's own Concept2 logbook (imported with `rowctl import-c2`, kept out of the repo) set
the priorities: long gaps undid progress, and strokes did little work at a normal rate.
So the two levers are **consistency** (no long gaps) and **power per stroke at a fixed
rate**. Headline numbers the app tracks:

1. **Watts at rate 20** (equivalently, joules per stroke). Rises only if strokes get better.
2. **Efficiency factor (EF)** = avg watts / avg HR on easy steady rows. Rises with aerobic fitness.
3. **Longest gap in the last 4 weeks** and **sessions per week**.

Useful formulas:

- Concept2 power from pace: `watts = 2.80 / (seconds_per_meter)^3`
- Work per stroke: `J/stroke = watts × 60 / spm`
- Distance per stroke: `m/stroke = distance / stroke_count`

---

## 2. Physical setup

```
 [screen]        ┌──────────── RowErg (~2.4 m) ────────────┐
 eye level ──▶   PM5 ◀── feet ════ seat rail ══════════════════
 past the PM5    └─────────────────────────────────────────┘
                                   ▲  ~3 m, square to the rail
                            [camera: iPhone]
                            lens ≈ seated hip–shoulder height
```

- **Camera side-on, screen in front.** A laptop's screen faces the same way as its camera, so
  one device can't do both. The camera watches from the side; a screen past the flywheel at
  eye level shows the rower their position without turning the head (a wall mirror forces a
  head turn, breaking the posture being checked).
- **The setup:** a spare iPhone side-on runs the camera page in Safari and does the pose work
  itself; only landmarks cross the Wi-Fi. The Linux laptop stands in front and runs
  `rowctl serve` from login (PM5 on USB, H10 over BLE); its browser shows the row screen at
  `http://localhost:8080`. The camera's setup check is also shown there, because the phone's
  screen faces away from the rower. Nothing is typed per row (§4, *One server*).
- **Browsers only open the camera on `localhost` or HTTPS.** A camera page on the server machine
  uses `localhost`. A phone uses HTTPS (`server::tls`): a local CA is made once in
  `<data>/tls/` (key mode 0600) and installed and trusted on the phone once; a server
  certificate is issued for the current LAN address, and again within 10 s whenever it changes
  (397 days, serverAuth, IP in the SAN: what iOS requires). The row screen shows a QR code
  (`rowctl` also prints it) for `http://<lan-ip>:8080/phone`, a setup page that checks whether
  the phone trusts the CA yet and links to the HTTPS camera page on `:8443`.
- **Only the laptop sees the data.** The server listens on the network for the phone, but
  another device gets only the app's files, the phone's setup and camera socket, and the
  token-guarded Watch upload; sessions, health and the row answer 403 (`server::local_only`).
- **Light beats pixels.** The phone's camera beats the laptop's webcam (720p MJPEG only) mainly in dim
  light: pose breaks on noise and motion blur, not resolution (the model works on a ~256 px
  crop). Whole body and full slide in frame; light from the camera side, never behind the rower.
- **Roles:** PM5 for numbers, front screen for posture, **audio** for cues.
- The same Linux laptop writes the code and runs the row.
- **PM5 over USB** (reliable, no BLE dropouts) to the laptop; it stands by the PM5, so a short
  cable does.
- **H10 over BLE** to the laptop. The H10 can also stay paired to the PM5 so the Concept2 log keeps HR.

---

## 3. Data sources

| Source | Transport | Signals | Rate | Used for |
|---|---|---|---|---|
| PM5 | USB HID, CSAFE | stroke state, watts, pace, spm, distance, drag factor, force curve; heart rate of a strap paired to it | ~20 Hz status; force curve per drive; HR 1 Hz | stroke boundaries (master clock), effort, force-curve technique; zones when the strap is paired to the PM5 |
| Polar H10 | BLE HR service (0x180D) + Polar PMD | HR, RR intervals; chest accelerometer (experimental) | per beat | zones, load, drift; HRV |
| Phone camera | Safari, MediaPipe Pose Landmarker | 33 body landmarks (near side used) | ~30 fps | posture, drive/recovery sequencing |
| Apple Watch | iPhone Shortcut → HTTP POST | overnight HRV (SDNN), resting HR, sleep | daily | readiness, Karvonen HR zones |
| Concept2 log | CSV import (later: Logbook API) | historic sessions | one-off / sync | baselines from May 2025 |

Video never leaves the browser; only landmark coordinates are sent to the server. The
exception is stills (640 pixels, the detected body drawn on), asked of the camera page one at a
time and served to the laptop only:
- For setting the camera up, the row screen asks for one every couple of seconds while "Show
  what the camera sees" is open; it's held in memory until the next one.
- During a row, `rowctl serve` saves one 5 s after the first stroke and then every 20 s (about
  60 in 20 minutes, landing on every part of the stroke), in a folder beside the recording:
  `<row>.stills/<µs>.jpg`, named by the session time it was asked at (the page draws the frame
  it has when the request arrives). They're for checking the detected body against the real
  one; the rower chose to keep them.

---

## 4. Architecture

```
 PM5 ─USB──┐                ┌────────── server (Rust, on the laptop) ───────────┐
 H10 ─BLE──┤                │ devices → event bus → stroke assembler → analysis │
 phone →   │   timestamped  │                       (1 record/stroke)  technique│
 browser ──┴──── events ──▶ │                                          effort   │
 (landmarks only)           │ recorder ─▶ session files        coach ─▶ audio   │
 iPhone Shortcut ─HTTP────▶ │ readiness        planner ─▶ next workout ─▶ PM5   │
                            │ SQLite                                            │
                            └───────────────────────────────────────────────────┘
```

**One server (`rowctl serve`, `rowctl::app`).** One process, started at login
(`contrib/systemd/`), runs the whole loop; the browser only shows it:
- The devices stay connected (both drivers reconnect on their own). While waiting, the last
  20 s of events are kept; the first stroke starts a recording with them, set up as the
  planner's row (preset, length, focus).
- A row ends after `--end-after` (120 s) without a stroke, or Stop on the page; a shutdown
  saves it too. Under 10 strokes is a bump of the handle and is dropped. The row is analysed
  and saved, the page opens its review, and the next row is planned (also at midnight, and
  when Watch data arrives while waiting: `Api::health_posted`).
- Besides the devices' events, the recording gets every cue the coach gave (`Cue`), the
  blocks the rower marks on the row screen (`Marker`, `POST /api/row/mark`), and the plan and
  focus in its header; camera stills go beside it (§3). On the review, the rower scores how
  hard it felt (1–10) and adds a comment (`PUT /api/sessions/{id}/feel`, stored on the
  session, kept when it's re-analysed): what the numbers can't say (sleep, a hot room, a
  sore back, a stop to fix the strap).
- State goes to the page on `/ws/live` every second (`{"type":"row",…}`, `server::row`), and
  `POST /api/row/stop` ends a row.
- The session clock counts with `CLOCK_BOOTTIME`, so a server running across suspend still
  dates the next morning's row correctly.
- `record` and `watch` still work for testing, with the server stopped (they need the PM5,
  the strap and port 8080).

### 4.1 Core principles

1. **The stroke is the unit.** PM5 stroke-state transitions (catch → drive → recovery)
   define stroke windows. Every other stream is sliced into those windows, producing one
   `StrokeRecord` per stroke.
2. **Record and replay.** Every session writes its raw, timestamped event stream to disk.
   Analysis is pure functions over events, so any past session can be re-run through new
   analysis code, and tests run against real recorded rows instead of a live erg.
3. **I/O at the edges.** Device drivers and HTTP live in their own crates; stroke assembly,
   technique, effort, readiness and planning have no I/O and are unit-tested on fixtures.

### 4.2 Events

Defined in `crates/model/src/event.rs`. Every event is `{ t_us, kind }`, where `t_us` is
wall-clock microseconds stamped on arrival by one session `Clock` (Unix time anchored
once, advanced monotonically).

| Kind | Status | Content |
|---|---|---|
| `Pm5Status` | done | stroke state, spm, watts, distance (dm), work time (cs), drag factor, workout state; each `Option` |
| `Pm5Force` | done | raw force samples (lbf), up to 16 per poll, plus stroke state |
| `Heart` | done | bpm, RR intervals (1/1024 s, as sent), skin contact |
| `Link` | done | device connected / dropped, with name or error |
| `Marker` | done | user annotation (deliberate-fault fixtures) |
| `Cue` | done | what the coach said, when |
| `H10Accel` | planned | chest accelerometer samples |
| `Pose` | done | capture time (browser clock), frame size, 9 joints × both sides as `[x, y, visibility]` (`model::POSE_JOINTS`) |
| coach/control | partly | cues are `Cue` events and the focus is in the header; session start/stop planned |

Session files (`crates/recording`): the 8-byte magic `ROWREC01`, then a zstd stream of
length-prefixed **MessagePack** frames with structs encoded as maps, so an optional field
can be added later without breaking old recordings. The writer flushes every second
(a crash loses ≤ 1 s) and the reader stops cleanly at a truncated tail. 12 s of simulated
PM5 + HR is 3.4 KB; with 30 fps pose a 20-min row is ~6 MB (simulated), almost all pose. Quantising landmarks to u16 would roughly halve that if it matters. `rowctl dump` prints JSON lines.

### 4.3 Stroke record

`analysis::strokes::StrokeAssembler` turns the event stream into one `StrokeRecord` per stroke
(catch to next catch). Built today:

```rust
struct StrokeRecord {
    index: u32,
    t_catch_us: i64, t_finish_us: Option<i64>, t_next_catch_us: Option<i64>,
    drive_ms, recovery_ms, ratio: Option<f64>,
    watts: Option<u16>, spm: Option<u16>,   // PM5's numbers for this stroke
    distance_m: Option<f64>,                // PM5 distance, this catch to the next
    drag_factor: Option<u8>,
    hr_bpm: Option<f64>,                    // mean over the stroke (≤ 5 s old reading as fallback)
    peak_n: Option<f64>,
    force_n: Vec<f64>,                      // resampled to 50 points, newtons
}
// rate() from the stroke's own timing; joules() = watts × stroke duration
```

A stroke's power and rate are taken from the last PM5 poll before the next catch, assuming the
PM5 reports them by the end of the recovery (to confirm on hardware). Phase 2 adds:

```rust
    pose: Option<StrokePose>,         // angle time series + keyframes
    faults: Vec<FaultHit>,

struct StrokePose {
    knee: Vec<f32>, elbow: Vec<f32>, trunk: Vec<f32>, shin: Vec<f32>, // degrees, per frame
    hip_x: Vec<f32>, wrist_x: Vec<f32>, wrist_y: Vec<f32>,           // normalised to body size
    catch: Keyframe, mid_drive: Keyframe, finish: Keyframe, mid_recovery: Keyframe,
}
```

Sessions and their strokes are stored in SQLite (`sessions`, `strokes`) as a cache of the
analysis: `rowctl analyze --all` rebuilds them from the recordings.

### 4.4 Time alignment

- The browser stamps each frame with its capture time (`performance.timeOrigin` + the video
  frame's `captureTime`) and converts it to the server's clock before sending: the camera page
  pings over `/ws/camera`, the server answers with its session time, and the quickest round
  trip of the last minute gives the offset (NTP-style; re-pinged every 5 s to follow drift
  between two crystals, ~10–50 ppm). On the same machine it's ~0; a phone's clock can be off by
  anything. The server also stamps arrival.
- A residual offset remains (camera latency, clock skew, the PM5's own detection delay).
  **Auto-calibrated** (`analysis::technique::sync_offset`) over strokes 4–23: the camera catch
  is the frame with the wrist furthest forward within −600/+300 ms of the PM5 drive start; the
  median difference is the session offset. Built; on the kinematic rower it recovers a
  simulated 60 ms lag within half a frame.
- No sub-frame peak fitting: the catch is lopsided (the drive leaves it ~5× faster than the
  recovery arrives), which drags a fitted parabola's peak toward the recovery by up to half a
  frame. The best frame alone has no bias.

### 4.5 Camera geometry

Built in `analysis::pose`; the camera page shows a live setup check from it (fps, person,
whole body in view, side-on, size in frame, landmark clarity).

- **Near side**: auto-detected from landmark visibility; only near-side joints are used.
- **Free horizon**: the hip slides along a level rail, so the hip path across a stroke is a
  horizontal line. Its slope gives camera tilt, corrected before computing angles. Only frames
  from the first stroke to the last (± 2 s) count (`technique::rowing_frames`): climbing on and
  off moves the hip in ways rowing doesn't: fitting every frame read a level camera as tilted by
  several degrees, and turned every body angle with it.
- **Scale**: distances are normalised by thigh length (hip–knee) so camera distance doesn't matter.
- Angles: trunk = hip→shoulder vs vertical (+ toward the feet); shin = ankle→knee vs vertical
  (+ knee toward the hips); knee = hip–knee–ankle; hip = shoulder–hip–knee; elbow =
  shoulder–elbow–wrist. Signs are relative to where the feet are, so either side to the camera
  gives the same numbers.
- Stroke figures (catch, finish) are stored in thigh lengths from the hip, for the review
  screen's stick figures and later the mirror.

---

## 5. Technique

### 5.1 Fault catalogue

Ordered by what makes a stroke weak, which is what the baseline points to.

| # | Fault | Detection | Source | Cue |
|---|---|---|---|---|
| 1 | **Short drive / incomplete compression** | Knee angle at catch too open; shins well short of vertical; short drive length | camera + PM5 | "Get to full compression" |
| 2 | **Legs not connected (shooting the slide)** | In the first third of the drive, hip moves back much further than the wrist; force curve spikes early then dips | camera + PM5 | "Handle moves with the seat" |
| 3 | **Early arms** | Elbow < ~160° while knee < ~150° (legs not yet down) | camera | "Legs first, arms long" |
| 4 | **Rushed recovery** | Recovery/drive < 1.5 at ≤ 24 spm (target ~2:1) | PM5 | "Slow the slide" |
| 5 | **Catch posture** | Trunk < ~15° forward at catch (no pivot from hips), or over-reach | camera | "Reach from the hips" |
| 6 | **Finish posture** | Layback > ~30° or < ~5°; wrist far above/below lower ribs | camera | "Sit tall, handle to ribs" |
| 7 | **Recovery order** | Knee bends before wrist passes the knee | camera | "Hands, body, slide" |
| 8 | **Force curve shape** | Double hump / mid-drive dip; peak very early or late; low stroke-to-stroke similarity | PM5 | review screen only |

Thresholds start from common coaching norms and are tuned against recorded sessions.
**Built (`analysis::technique`):** 1 = shin > 12° from vertical at the camera catch; 2 = trunk
tips forward > 6° past its catch angle in the first third of the drive (if the hips leave and
the handle doesn't, the trunk must tip: cleaner than comparing hip and wrist travel); 3 = elbow
< 160° while the knee is still < 150°; 4 = recovery/drive < 1.5 at ≤ 24 spm, by the camera's
handle timing when there is one, else the PM5's (`technique::handle_timing`). The PM5's drive
time only covers the flywheel speeding up, about half the handle's drive, so its ratio reads
more than twice the real one. Session drive time and ratio, and the review's, use
the camera's when it saw most strokes (`Summary::handle_timing`, analysis version 2). Each is tested against the simulator's kinematic rower
(`rowctl simulate --fault early-arms` etc.): the injected fault is found on > 90% of strokes
and nothing fires on clean rowing. Real deliberately-rowed fixtures are still to come.
Every fault has a matching drill (legs-only, pick drill, pause at hands-away, rate ladder).

### 5.2 Coach behaviour

- **One focus per session**, picked by the planner from the most frequent fault in recent sessions.
  Other faults are recorded but not spoken.
- Speak when the focus fault appears in **≥ 6 of the last 10 strokes**; then wait **≥ 30 s**.
- Positive cue when the rate drops below 2/10 for 20 strokes ("That's it.").
- Cues are ≤ 4 words. Spoken by the OS speech command for now (`spd-say` or `espeak-ng`); browser
  `speechSynthesis` once the live view moves into the browser.

**Built (`analysis::cues::FocusCoach`, `analysis::technique::LiveTechnique`):** each stroke gets
its camera view and faults as it completes (sync calibrates live on strokes 4–23 and agrees
with the after-the-fact analysis from then on). `rowctl record --focus auto|none|<fault>`;
auto picks the fault with the highest rate (≥ 10%) over the last 3 real sessions, camera faults
only when the camera is on. The focus cue outranks pacing cues: at most one cue per stroke.
- Effort cues (HR zone) share the same queue with lower priority.

**Pacing cues (Phase 1, built):** `rowctl record --plan reference --speak` (or `--rate`, `--watts`,
`--zone`, `--minutes`) speaks short corrections through the OS speech command (`spd-say` or `espeak-ng`).
Judged on the median of the last 6 strokes; silent for the first 60 s; ≥ 30 s between cues and
≥ 60 s before the same cue repeats; priority heart rate > rate > power, and a held-back
heart-rate cue blocks the power cue (never "harder" while HR is over the ceiling). Halfway and
time calls always get through.

### 5.3 Mirror (visual feedback)

The front screen shows the rower's own position, drawn from landmarks (no video streaming):

- **Live stick figure** with target guides: a shin-vertical line and body-angle wedge for the
  catch, a layback wedge for the finish.
- **Last stroke frozen** at catch and finish beside the live figure, with a faint ghost of a
  reference stroke. At 20 spm a stroke lasts 3 s, too fast to judge a moving figure, but a
  frozen catch reads at a glance.
- **Used in doses, not all the time.** Constant augmented feedback creates dependence: form
  holds while it's on and slips when it's off (the guidance hypothesis). Mirror on for drills,
  warm-up and the start of a focus block; audio only during steady pieces; regular
  no-feedback blocks to check the fix stuck. The planner schedules these.

**Built:** the row screen (`/`, also `/mirror`), fed by `/ws/live` from `rowctl serve` (or
`record --camera`), sized to fit one laptop screen read from the erg:
- **Live figure** drawn from where the feet rest (the median ankle, `LiveTechnique::figure`),
  so it holds still while the seat slides; framed to the rower's own range of movement.
- **Target rower** behind it (`frontend/src/lib/ghost.js`): a good stroke drawn with the
  rower's own proportions, measured from the live figures (thigh, shin, trunk, arm lengths,
  seat and handle heights). Drive: legs, then the body swings from the catch lean to the finish
  lean (the middles of `CATCH_TRUNK_GUIDE` and `FINISH_TRUNK_GUIDE`), then the arms; recovery:
  arms, body, then a slow slide to shins vertical. It restarts at each of the rower's catches
  (the wrist's furthest forward point, found in the live figure itself, so it shares the
  figure's delay) and runs at the target rhythm (`cues::Rhythm`: a third of the stroke on the
  drive, 0.8–1.1 s; 1 s + 2 s at rate 20). Ahead of it means rushing; past its own stroke it
  waits at the catch.
- **Rhythm bar**: the target drive and recovery with a marker for now, and the last stroke's,
  by the camera; "Slow the slide" when the recovery is 0.25 s or more short.
- **Numbers against targets** (`LiveMsg::Targets`, every second): rate, watts and heart rate,
  each with its band and which way to go. Heart rate first: "harder" isn't shown while it's
  over the ceiling.
- Beside it, the last stroke frozen at catch and finish over the median of the last 20 strokes;
  the coach's cue as a banner; a "Numbers only" switch for the no-feedback blocks. Only figures
  travel, never video.

**Validation:** each fault detector ships with a recorded session where the fault is rowed
*on purpose* (e.g., 20 strokes bending the arms early). The detector must flag those strokes
and stay quiet on clean ones.

**Built:** on the row screen, "Mark faults on purpose…" opens a button per block (Clean, or
one of the four faults); press one as each block starts. A mark before the first stroke
starts the row with it. During `rowctl record`, type `0` (clean) or `1`–`4` (short
compression, shooting the slide, early arms, rushed recovery) + Enter instead. Either way
they're stored as `Marker` events. `rowctl check <file>` scores every block (`analysis::fixtures`), skipping
its first 2 strokes: a fault block passes if the focus coach would have cued that fault, a
clean block if it would have cued nothing, with each fault's flag rate alongside. Suggested
fixture: 2 min clean, 2 min of one fault, 2 min clean, per fault.

---

## 6. Effort and fitness

- **Zones**: Karvonen, `HRrest + p × (HRmax − HRrest)`, zone floors at 50/60/70/80/90% of reserve.
  HRrest is set by hand (assumed 60 until then; the 14-day Watch median in Phase 3). HRmax is
  set by hand or estimated as `208 − 0.7 × age` (age from the logbook). When a session's
  highest 10 s mean exceeds it, the report suggests raising it; it doesn't raise it itself,
  because a dry strap at the start of a row can read high for many seconds.
- **Live**: zone on every stroke line, HR ceiling cue (`--zone N` or the easy plan).
- **Per session** (`analysis::session::Summary`):
  - Power from **average pace** (as the PM5 and logbook compute it), not the mean of stroke watts.
  - Banister **TRIMP**, sex-specific exponent (profile setting; the male one when unset).
  - **EF** = watts / HR.
  - **Aerobic decoupling** = EF drop from first to second half, skipping the first 3 min (HR lag)
    and needing 10 min after that; < 5% means aerobic fitness covers that duration.
  - **Watts at rate 20** for strokes at 19.5–20.5 spm by their own timing (the PM5's spm is a
    whole number), needing 2 min of them.
  - Last 10 minutes as its own segment (sessions ≥ 15 min). Strokes spanning a pause (> 6 s)
    are left out of all of these.
- **Long term**: fitness/fatigue/form (42-day and 7-day EWMA of load, and their difference),
  EF trend, watts-at-20 trend, fault-rate trend per fault, **form under fatigue**
  (fault rate vs HR zone and minutes rowed).

---

## 7. Readiness (Apple Watch)

- An iPhone Shortcut (time-of-day automation) reads the last **7 days** of HRV, resting HR and sleep
  and POSTs them to `POST /api/health/daily` on the laptop. The server upserts by date, so a run
  missed while the laptop is off is backfilled by the next one.
- Auth: a shared token header (LAN only).
- Readiness: ln(HRV) 7-day mean vs the 60-day band, resting HR vs baseline, last night's sleep
  → green / amber / red. Rules are explicit and tunable, not a black box.

**Built:** `POST /api/health/daily` (`crates/server/src/health.rs`, token in
`<data>/api-token`, constant-time compare) takes raw samples and aggregates them per day
(`store::health`): mean HRV, mean resting HR, and sleep as the union of asleep intervals
(phone and Watch overlap) counted for the morning it ended. `analysis::readiness`:

| Signal | Amber | Red |
|---|---|---|
| ln(HRV), 7-day mean vs the 60 days before (±1 SD band, ≥ 14 days) | below the band | > 2 SD below |
| Resting HR, 7-day mean vs the 30 days before | > +5 bpm | > +8 bpm |
| Last night's sleep | < 6 h | < 5 h |

One amber → amber; two ambers or any red → red. Shown at the start of `rowctl record`, by
`rowctl health`, and on the review screen's sidebar. HRrest for the zones is the two-week
median Watch resting HR (after a week of data) unless set by hand. Shortcut setup:
`docs/SHORTCUT.md` (untested on a phone).

---

## 8. Planner (the improvement loop)

**Built (`analysis::planner`, `rowctl plan`):** the rules below, over every row so far (logbook
and app sessions, uploads of the same row counted once) and today's readiness. A gap over 21
days starts a block with a reference row; its verdict sets the comeback ramp (1/2/3 weeks of
easy rows); then build weeks: rate steps once a week after a steady row, otherwise steady
rows, a lighter fourth week, the rate-20 test after 3 build weeks (once per block). A gap of
8–21 days turns intensity into an easy week. Amber → no intensity; red → minimum session.
`rowctl plan` prints the session, the reasons, and the exact `rowctl record --plan …` command;
the presets record their name (`reference`, `easy`, `steady`, `rate20`, `rate_steps`) so the
planner knows what was rowed.

### 8.1 Consistency rules

- Main rule: **no gap longer than 7 days**.
- After a gap of 8–21 days: one easy week before intensity. Longer than 21 days: full comeback ramp.
- Low readiness or no time → offer a **minimum session** (15–20 min easy) instead of skipping.
- Streaks count **weeks with ≥ 3 sessions**, not days.
- Optional phone nudge at 4 days without a row.

### 8.2 Comeback ramp (self-calibrating)

Fitness after a long gap is unknown, so the ramp measures it:

1. First session is a **reference row**: `rowctl record --plan reference --speak`, 20 min at rate
   20, ~50–55 W, the same drag factor every time.
2. Compare the session's EF with past steady pieces **at similar power** (EF climbs with
   watts, so a harder row looks fitter): ≥ 15 min, rate 16–22, within ±8 W, from the
   logbook and earlier app sessions. Whole-session EF is compared, because logbook EF is a
   whole-piece average too.
3. The bands come from that history: mean of the best 3 comparable pieces and
   of the worst 3. EF ≥ best → fitness held, 1-week ramp. EF ≤ worst → full 2–3 weeks.
   Between → 2 weeks. Needs ≥ 4 comparable pieces.
4. Ramp week: 3 × 20–25 min easy at rate 18–20 (`--plan easy`: HR under zone 2's top), no intervals.

### 8.3 Weekly template after the ramp

- 2 × steady: zone 2, 30–45 min, rate 18–20, technique focus.
- 1 × rate/intensity work: e.g. 4 × 5 min stepping rate 20/22/24 at matched form, later 6 × 500 m.
  Intensity only goes up once fault rates hold at that rate.
- 1 × optional technique session: drills plus a rate ladder.
- Volume +≤10%/week; lighter week every 4th week. Amber readiness → no intensity; red → minimum session.

### 8.4 Setup protocol

- **Fixed drag factor**: chosen once (start ~110). The setup screen warns when it's more than ±5 off.
- Planner shows the workout; loading it onto the PM5 over CSAFE is a goal, not a given.
  v1 found that `SETPMCFG` workout sub-commands return PREVREJECT without Concept2's USB
  auth; only the screen-state command was accepted. The standard CSAFE workout commands
  (`SETTWORK`, `SETHORIZONTAL`, `SETPROGRAM`) are untested. Fallback: set it on the PM5 by hand.
- **Experiment ready:** `rowctl pm5-probe --workout` reads the workout type, sends
  SETTWORK, SETPROGRAM 0 and GOINUSE one frame each (so a rejection names the command), reads
  back, then the same in one frame, and reads back again. Fixed time (type 4/5) afterwards
  means it works; the frames (`pm5::fixed_time_workout`, `fixed_distance_workout`) are
  byte-tested.

### 8.5 Benchmarks

- **Rate-20 test**: 20 min held at exactly 20 spm, best steady average watts. First after 3
  consistent weeks. First milestone **100 W**
  (~300 J/stroke, ~9.9 m/stroke).
- **2k**: around week 8, then every ~6 weeks alongside the rate-20 test.

---

## 9. Screens

Visual language from `.impeccable.md`: dark, instrument-panel, amber accent, Chakra Petch numerals.

1. **Row screen** (`/`, the laptop in front, built): before the first stroke, the next row's
   plan, cues and readiness, the devices, and the phone's QR code while no camera is
   connected; while rowing, the mirror (§5.3) with recording time, strokes, a countdown once
   resting, and Stop. A saved row opens its review; a row starting brings the screen back.
   Later: drag factor, current fault rate, force curve trace.
2. *(merged into 1)*
3. **Session review** (laptop, built: `rowctl serve`): readout strip (time, metres, pace, watts,
   spm, HR, EF) and secondary numbers; time in zones as a segmented bar; power, rate and HR per
   stroke as small multiples on one time axis with a shared crosshair; the selected stroke's
   force curve against the session mean, with jumps to the most- and least-work strokes; every
   stroke as a table. Amber always means "the selected stroke". Steady rows of 15+ min show
   their EF against past pieces at similar power, and a reference row its ramp verdict. The
   sidebar carries readiness and **Today** (the planner's session and reasons; `/api/today`).
4. **Progress** (built: `/progress`, `analysis::progress`): watts at 20 against the 100 W
   milestone (app strokes at 19.5–20.5 spm; logbook pieces averaged at 20 spm, shown faint),
   EF of comparable steady rows (≥ 15 min, 16–22 spm, 43–63 W only: EF climbs with power),
   fitness/fatigue (42/7-day averages of TRIMP; logbook load estimated from average HR) and
   form as a separate chart, rows per week against the aim of 3 with the streak, fault rates
   per camera session, form under fatigue (first vs second half of the last 10 camera
   sessions), benchmarks (rate-20 tests, 2k pieces).

---

## 10. Stack and layout

```
crates/
  model/      events, session clock (no I/O)
  devices/    csafe framing, pm5 (hidapi), h10 (btleplug: HR + RR, PMD later), simulator
  recording/  session files: write, read, tolerate truncation
  analysis/   stroke records, session summary, zones + TRIMP, reference check, pacing cues;
              later technique, readiness, planner (pure)
  store/      data paths, SQLite (sqlx, migrations), Concept2 CSV importer
  rowctl/     CLI: record, watch, analyze, sessions, profile, simulate, dump, replay, curves,
              pm5-probe, import-c2, history
  server/     axum: JSON API, camera and live sockets, phone HTTPS, row state (`/api/row`), the frontend
frontend/     Svelte 5 + Vite: review (built); later setup, camera (@mediapipe/tasks-vision), live, progress
migrations/
contrib/udev/ hidraw access rule for the PM5
contrib/systemd/ user service: `rowctl serve` at login
fixtures/     recorded sessions used by tests
docs/
```

Data lives in `$ROW_ASSIST_DATA` or `~/.local/share/row-assist/` (`row-assist.db`, `sessions/`).

Taken from v1 (git history): CSAFE framing and checksum, HID report layout and interface
selection (usage page 0xFF00, report ID 0x02), the start-workout frame and its byte test, the
udev rule. **Not** taken: v1's byte stuffing (`b ^ 0x03`; the spec is `b - 0xF0`, see §12) and
its command mapping (e.g. bare `0xA6` read as stroke state; in standard CSAFE it's GETPACE).
Commands now follow the published CSAFE/PM tables, with PM-specific ones wrapped in `0x1A`.

---

## 11. Phases

Each phase ends with something usable on the erg.

| Phase | Scope | Done when |
|---|---|---|
| **0. Foundations** | workspace; event model; PM5 driver port + drag factor + **force curve spike**; H10 HR + RR; recorder, replay, dump; Concept2 CSV importer | a real row is recorded, replayed, and its force curves dumped; the CSV is in SQLite. *Status: code done and verified with the simulator; CSV imported. Waiting on hardware: `pm5-probe`, then a real `record`.* |
| **1. Effort** | stroke assembler; session persistence; zones, TRIMP, EF, decoupling, watts at 20; drag-factor check; review screen; audio zone cues; reference-row comparison | comeback-ramp sessions are rowed with the app. *Status: built: `rowctl record --plan … --speak`, `analyze`, `sessions`, `profile`, `simulate`, and the review screen (`rowctl serve`). Waiting on the first real comeback-ramp rows.* |
| **2. Posture** | camera page; framing check; near-side + horizon + scale; sync auto-calibration; joint angles; faults 1–4; one-focus coach; **mirror on the front screen**; deliberate-fault fixtures | the coach flags deliberately rowed faults and stays quiet on clean strokes. *Status: built: camera page (`rowctl record --camera`), setup check, near side, tilt, scale, sync, joint angles, faults 1–4, technique on the review screen, the one-focus coach (live), a kinematic rower in the simulator (`--sim-fault`). the mirror on the front screen. Next: real deliberately-rowed fixtures and threshold tuning, which need rows on the erg.* |
| **3. Readiness** | Shortcut endpoint; baselines; readiness on setup screen; Karvonen zones from Watch resting HR | a week of Watch data shows up and changes the zones. *Status: built and tested with synthetic data; waiting on the Shortcut (docs/SHORTCUT.md).* |
| **4. Improvement loop** | planner; PM5 workout programming; progress screen; form under fatigue; benchmarks | the app proposes next session + focus and loads it onto the PM5. *Status: planner + benchmark scheduling (`rowctl plan`) and the progress screen built; PM5 programming needs hardware.* |
| **Later** | H10 accelerometer; faults 5–8; front camera view (symmetry); optional video capture for review; Concept2 Logbook API sync (incl. stroke data if present); weekly written summary | — |

---

## 12. Risks and open questions

- **Force curve throughput**: the force plot command returns up to 16 points per request and must be
  polled through the drive alongside status polling. First spike in Phase 0; fallback is the
  PM5 BLE force-curve characteristic.
- **Pose quality**: webcam at 30 fps gives ~25 frames per drive at 20 spm. Good enough for
  sequencing, only coarse for catch timing. Lighting and laptop placement matter most.
- **The phone camera is untested on a real iPhone.** An emulated phone passed (HTTPS, clock
  offset, ~26 fps on the GPU); still unknown: installing the CA profile, MediaPipe on Safari's
  GPU (it falls back to the CPU), the wake lock (iOS 16.4+) and heat over a long row.
- **PM5 command codes and byte order** come from the published tables, not hardware.
  `rowctl pm5-probe` sends each command alone and prints both byte orders; run it idle and
  while rowing before trusting distance, time and force values.
- **v1's byte stuffing was wrong** (`b ^ 0x03` turns `0xF1` into `F3 F2`, a STOP flag). Fixed,
  with a test over all 256 byte values. That bug likely caused v1's "intermittent checksum
  failures", and would have corrupted ~40% of force-curve replies.
- **PM5 workout programming over CSAFE**: v1 got PREVREJECT for workout sub-commands (see
  §8.4); the standard-command experiment is ready (`rowctl pm5-probe --workout`) and needs a
  PM5.
- **H10 connections**: while the H10 is paired to the PM5, the laptop doesn't see it at all (2026-11-06).
  So the PM5 passes on its heart rate (CSAFE `GETHRCUR`, answered 84 bpm on real hardware) as
  `Heart { via_pm5 }`: bpm only, no RR intervals. A strap connected to the laptop directly wins and
  the relayed beats are dropped (`HeartPick`). Nothing uses RR intervals yet; HRV during a row would
  need the strap paired to the laptop instead.
- **HR max** is unknown; zones stay provisional (age estimate) until set or a hard effort is recorded.
- **Which PM5 poll carries a stroke's power and rate** is assumed (the last one before the next
  catch). A real recording will show whether the value changes at the end of the drive or at
  the catch; if at the catch, the assembler must take it from the poll after.

## 13. References

- Concept2 forum, force plot data command: https://c2forum.com/viewtopic.php?f=15&t=83
- Polar PMD in Rust (`arctic` crate): https://docs.rs/crate/arctic/latest/source/src/control.rs
- Polar BLE SDK (PMD spec PDF in repo): https://github.com/polarofficial/polar-ble-sdk
