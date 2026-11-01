# Apple Watch data via an iPhone Shortcut

Row Assist reads HRV, resting heart rate and sleep from Apple Health through a
personal-automation Shortcut that posts the last 7 days to the Mac every morning. Each
run resends the whole week, so a morning the Mac was asleep is filled in by the next run.

> **Untested.** This was written without an iPhone to hand. The server side is tested; the
> Shortcut steps below follow how Shortcuts' Health actions work, but names can differ
> between iOS versions. The server tells you what it didn't like (see *Checking it*).

## On the Mac

1. Get the token: `rowctl health --token`.
2. Serve on the network while the Shortcut runs: `rowctl serve --addr 0.0.0.0:8080`
   (or record with `--listen 0.0.0.0:8080`). Allow incoming connections if macOS asks.
   Everything the server shows becomes readable on your home network; only posting
   Watch data needs the token.
3. The Mac's address: System Settings → General → Sharing → Local hostname, e.g.
   `my-mac.local`.

## The Shortcut

Create a new Shortcut named **Row Assist health** with these actions:

1. **Find Health Samples** where *Type* is *Heart Rate Variability*, *Start Date* is in the
   last 7 days. Sort by *Start Date*, no limit.
2. **Repeat with Each** item in *Health Samples*:
   - **Format Date** of *Repeat Item → Start Date*, format **ISO 8601** (include time).
   - **Dictionary** with `date` = *Formatted Date*, `value` = *Repeat Item → Value*.
   - **Add to Variable** `HRV`.
3. The same for *Resting Heart Rate*, adding to `Resting`.
4. **Find Health Samples** where *Type* is *Sleep Analysis*, *Start Date* in the last 7 days.
   **Repeat with Each**: format *Start Date* and *End Date* as ISO 8601, then a **Dictionary**
   with `start`, `end` and `value` = *Repeat Item → Value*; **Add to Variable** `Sleep`.
5. **Dictionary** with `hrv` = `HRV`, `resting_hr` = `Resting`, `sleep` = `Sleep`
   (each as an array).
6. **Get Contents of URL**: `http://<mac-hostname>:8080/api/health/daily`, *Method* POST,
   *Headers*: `Authorization` = `Bearer <token>`, *Request Body*: *File* = the Dictionary
   (it is sent as JSON).

Then **Automation → Create Personal Automation → Time of Day**, e.g. 09:00 daily, run
**Row Assist health**, with *Ask Before Running* off.

## What gets sent

```json
{
  "hrv":        [{"date": "2026-11-01T03:12:00+02:00", "value": 48.2}],
  "resting_hr": [{"date": "2026-11-01T08:00:00+02:00", "value": 56}],
  "sleep":      [{"start": "2026-10-31T23:40:00+02:00", "end": "2026-11-01T06:50:00+02:00", "value": "Core"}]
}
```

- Dates: ISO 8601, `2026-11-01 03:12:00 +0200` (Health export style), or `2026-11-01`.
- Values may be numbers or text that starts with one (`"48 ms"`).
- Sleep: every stage counts as asleep except *In Bed* and *Awake*; overlapping samples
  from the phone and the Watch are merged, and a night counts for the morning it ended.

## Checking it

- Run the Shortcut by hand. The server answers with the stored days and today's readiness;
  an error names the problem (`401`: token; `400`: what couldn't be read).
- `rowctl health` on the Mac lists the last two weeks and today's readiness.
- To test without the phone: save a JSON file in the format above and
  `rowctl health --import file.json`.

Readiness needs about two weeks of data before it says more than "no baseline yet"; the
zones switch to the Watch's resting HR after one week (unless `rowctl profile --hr-rest`
is set, which wins).
