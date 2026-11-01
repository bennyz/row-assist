<script>
  // The long view: is power per stroke rising, is the aerobic base growing,
  // is the rowing consistent, does technique hold as you tire?
  import { getProgress } from './api.js';
  import { FAULTS, clock, fixed } from './format.js';
  import DayChart from './DayChart.svelte';

  const MILESTONE_W = 100;
  const WEEK_TARGET = 3;

  let p = $state(null);
  let error = $state(null);
  getProgress()
    .then((x) => (p = x))
    .catch((e) => (error = e.message));

  const x0 = $derived(p?.load[0]?.day ?? p?.weeks[0]?.start_day ?? 0);
  const x1 = $derived(p?.today ?? 0);
  const css = (name) => `var(${name})`;

  const w20 = $derived(
    p ? [{ key: 'w20', label: 'Watts at rate 20', color: css('--ink'), kind: 'dots', points: p.watts_at_20.map((q) => ({ ...q, faint: q.from_logbook, note: q.from_logbook ? 'logbook, whole piece at 20 spm' : 'app' })) }] : [],
  );
  const ef = $derived(
    p ? [{ key: 'ef', label: 'EF', color: css('--ink'), kind: 'dots', points: p.ef.map((q) => ({ ...q, note: `${fixed(q.context)} W` })) }] : [],
  );
  // Weekly samples keep the load lines light; the last day is always included.
  const sampled = $derived(p ? p.load.filter((d, i) => i % 7 === 0 || i === p.load.length - 1) : []);
  const load = $derived([
    { key: 'fitness', label: 'Fitness', color: css('--ink'), kind: 'line', points: sampled.map((d) => ({ day: d.day, value: d.fitness })) },
    { key: 'fatigue', label: 'Fatigue', color: css('--mean'), kind: 'line', points: sampled.map((d) => ({ day: d.day, value: d.fatigue })) },
  ]);
  const form = $derived([{ key: 'form', label: 'Form', color: css('--ink-2'), kind: 'line', points: sampled.map((d) => ({ day: d.day, value: d.form })) }]);
  const faultSeries = $derived(
    p
      ? Object.keys(FAULTS)
          .map((f) => ({ f, points: p.faults.filter((q) => q.fault === f).map((q) => ({ day: q.day, value: q.rate * 100 })) }))
          .filter((s) => s.points.length)
      : [],
  );

  const now = $derived(p?.load.at(-1));
  const thisWeek = $derived(p?.weeks.at(-1));
  const lastW20 = $derived(p?.watts_at_20.at(-1));
  const lastEf = $derived(p?.ef.at(-1));
  const weekMax = $derived(Math.max(WEEK_TARGET + 1, ...(p?.weeks.map((w) => w.rows) ?? [0])));
  const day = (d) => new Date(d * 86400000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric', timeZone: 'UTC' });
</script>

{#if error}
  <p class="error">Couldn't load progress: {error}</p>
{:else if !p}
  <p class="muted">Loading…</p>
{:else if !p.weeks.length}
  <p class="muted">No rows yet. Import the logbook (<code>rowctl import-c2</code>) or record a session.</p>
{:else}
  <h1>Progress</h1>
  <dl class="readout">
    <div><dd class="num">{fixed(lastW20?.value)}</dd><dt>watts at rate 20{lastW20 ? ` · ${day(lastW20.day)}` : ''}</dt></div>
    <div><dd class="num">{fixed(lastEf?.value, 3)}</dd><dt>EF, steady{lastEf ? ` · ${day(lastEf.day)}` : ''}</dt></div>
    <div><dd class="num">{thisWeek?.rows ?? 0}<span class="of">/{WEEK_TARGET}</span></dd><dt>rows this week</dt></div>
    <div><dd class="num">{p.streak_weeks}</dd><dt>weeks in a row at {WEEK_TARGET}+</dt></div>
    <div><dd class="num">{fixed(now?.form)}</dd><dt>form (fitness − fatigue)</dt></div>
  </dl>

  <section>
    <h2>Watts at rate 20 <span class="muted">· power per stroke; first milestone {MILESTONE_W} W</span></h2>
    {#if w20[0].points.length}
      <DayChart series={w20} {x0} {x1} unit="W" refs={[{ value: MILESTONE_W, label: `${MILESTONE_W} W` }]} />
      <p class="note muted">Faint: logbook pieces rowed at 20 spm (whole-piece average). Solid: app sessions, strokes at 19.5–20.5 spm.</p>
    {:else}
      <p class="muted">No rows at rate 20 yet.</p>
    {/if}
  </section>

  <section>
    <h2>Efficiency <span class="muted">· watts per heartbeat, steady rows at {p.ef_watts[0]}–{p.ef_watts[1]} W</span></h2>
    {#if ef[0].points.length}
      <DayChart series={ef} {x0} {x1} unit="W/bpm" digits={3} />
    {:else}
      <p class="muted">No comparable steady rows yet.</p>
    {/if}
  </section>

  <section>
    <h2>Training load <span class="muted">· TRIMP, 42-day and 7-day averages</span></h2>
    <DayChart series={load} {x0} {x1} zero />
    <h3>Form</h3>
    <DayChart series={form} {x0} {x1} height={110} refs={[{ value: 0, label: '0' }]} />
  </section>

  <section>
    <h2>Rows per week <span class="muted">· aim {WEEK_TARGET}</span></h2>
    <div class="weeks" role="img" aria-label="Rows each week; aim {WEEK_TARGET}">
      <span class="target" style:bottom="{(WEEK_TARGET / weekMax) * 100}%"><span>{WEEK_TARGET}</span></span>
      {#each p.weeks as w (w.start_day)}
        <span class="week" title="Week of {day(w.start_day)}: {w.rows} rows, {fixed(w.minutes)} min">
          {#if w.rows}<span class="bar" style:height="{(w.rows / weekMax) * 100}%"></span>{/if}
        </span>
      {/each}
    </div>
    <p class="note muted">{day(p.weeks[0].start_day)} to now · hover a week for its rows and minutes</p>
  </section>

  {#if faultSeries.length}
    <section>
      <h2>Faults <span class="muted">· share of strokes, camera sessions</span></h2>
      {#each faultSeries as s (s.f)}
        <h3>{FAULTS[s.f].label}</h3>
        <DayChart series={[{ key: s.f, label: FAULTS[s.f].label, color: css('--ink-2'), kind: 'both', points: s.points }]} {x0} {x1} height={100} unit="%" zero />
      {/each}
    </section>
  {/if}

  {#if p.fatigue.length}
    <section>
      <h2>Form under fatigue <span class="muted">· first vs second half of the last {p.fatigue[0].sessions} camera sessions</span></h2>
      <table class="tab">
        <thead><tr><th class="left">fault</th><th>first half</th><th>second half</th></tr></thead>
        <tbody>
          {#each p.fatigue as f (f.fault)}
            <tr>
              <td class="left">{FAULTS[f.fault]?.label}</td>
              <td>{fixed(f.first_half * 100)}%</td>
              <td class:worse={f.second_half > f.first_half + 0.1}>{fixed(f.second_half * 100)}%</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/if}

  <section>
    <h2>Benchmarks</h2>
    {#if p.benchmarks.length}
      <table class="tab">
        <thead><tr><th class="left">date</th><th class="left">test</th><th>result</th></tr></thead>
        <tbody>
          {#each p.benchmarks as b (b.day + b.kind)}
            <tr>
              <td class="left">{day(b.day)}</td>
              <td class="left">{b.kind === 'rate20' ? 'Rate-20 test' : '2k'}</td>
              <td>{b.kind === 'rate20' ? `${fixed(b.value)} W` : clock(b.value)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="muted">None yet. The planner schedules the rate-20 test after three steady weeks.</p>
    {/if}
  </section>
{/if}

<style>
  h1 {
    font-size: 20px;
    font-weight: 600;
    margin: 0;
  }
  h2 {
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-3);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 0 0 0.75rem;
  }
  h2 .muted {
    text-transform: none;
    letter-spacing: 0;
  }
  h3 {
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-2);
    margin: 1rem 0 0.25rem;
  }
  section {
    margin-top: 2.25rem;
  }
  .readout {
    display: flex;
    flex-wrap: wrap;
    margin: 1.25rem 0 0;
    border-top: 1px solid var(--rule);
    border-bottom: 1px solid var(--rule);
  }
  .readout > div {
    padding: 0.9rem 1.6rem 0.8rem 0;
    margin-right: 1.6rem;
    border-right: 1px solid var(--rule);
  }
  .readout > div:last-child {
    border-right: 0;
  }
  .readout dd {
    margin: 0;
    font-size: 40px;
    font-weight: 600;
    line-height: 1;
  }
  .readout .of {
    font-size: 22px;
    color: var(--ink-3);
  }
  .readout dt {
    margin-top: 0.4rem;
    font-size: 12px;
    color: var(--ink-3);
  }
  .note {
    font-size: 12px;
    margin: 0.4rem 0 0;
  }
  .weeks {
    position: relative;
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 90px;
    border-bottom: 1px solid var(--rule);
  }
  .week {
    flex: 1 1 0;
    max-width: 24px;
    height: 100%;
    display: flex;
    align-items: flex-end;
  }
  .week:hover .bar {
    background: var(--ink);
  }
  .bar {
    width: 100%;
    background: var(--ink-2);
    border-radius: 4px 4px 0 0;
  }
  .target {
    position: absolute;
    left: 0;
    right: 0;
    border-top: 1px solid var(--ink-3);
    pointer-events: none;
  }
  .target span {
    position: absolute;
    right: 0;
    top: -1.2rem;
    font-size: 11px;
    color: var(--ink-3);
  }
  table {
    border-collapse: collapse;
    font-size: 14px;
    min-width: 22rem;
  }
  th {
    color: var(--ink-3);
    font-weight: 500;
    text-align: right;
    padding: 0.3rem 0.8rem;
    border-bottom: 1px solid var(--rule);
  }
  td {
    text-align: right;
    padding: 0.3rem 0.8rem;
    border-bottom: 1px solid var(--rule);
    color: var(--ink-2);
  }
  .left {
    text-align: left;
  }
  td.worse {
    color: var(--ink);
    font-weight: 600;
  }
  .error {
    color: var(--bad);
  }
  code {
    font-family: var(--font-num);
  }
</style>
