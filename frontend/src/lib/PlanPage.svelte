<script>
  // The plan: where the block stands, this week's rows, the rows ahead if
  // each is rowed as proposed, and the power from last time (the interval
  // workouts' and the easy row's). It all re-plans after every row and with each day's readiness.
  import { getPlan } from './api.js';
  import { KINDS, PLANS, READINESS, minutesLabel } from './format.js';

  let p = $state(null);
  let error = $state(null);
  getPlan()
    .then((x) => (p = x))
    .catch((e) => (error = e.message));

  const date = (d) =>
    new Date(d * 86400000).toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short', timeZone: 'UTC' });
  const PHASES = {
    restart: () => 'Restart: a reference row to measure where fitness is',
    comeback: (ph) => `Comeback ramp, week ${ph.week} of ${ph.of}: easy rows only`,
    easy_week: () => 'An easy week after a gap, before intensity',
    build: (ph) => `Build, week ${ph.week}: two steady rows and one hard row a week`,
  };
  const phaseLabel = (ph) => PHASES[ph.phase]?.(ph) ?? ph.phase;
  // The reference row and the test are fixed pieces; their name says so.
  const length = (a) => (['reference', 'rate20_test'].includes(a.kind) ? '' : minutesLabel(a.minutes));
  // The rows ahead, by the week they fall in (from today).
  const weeks = $derived(
    p
      ? Object.values(
          Object.groupBy(p.ahead, (a) => Math.floor((a.day - p.today) / 7)),
        )
      : [],
  );
  const now = $derived(p?.ahead[0]);
  const moved = $derived(p && (p.movement.active_min > 0 || p.movement.strength_days > 0 || p.movement.steps_per_day != null));
  const ready = $derived(READINESS[p?.readiness.level] ?? READINESS.unknown);
</script>

{#if error}
  <p class="error">Couldn't load the plan: {error}</p>
{:else if !p}
  <p class="muted">Loading…</p>
{:else}
  <h1>Plan</h1>
  <p class="phase">{phaseLabel(now.phase)}</p>

  <section>
    <h2>This week <span class="muted">{p.done.length} of {p.target_per_week} rows in the last 7 days</span></h2>
    {#if p.done.length}
      <ul class="rows">
        {#each p.done as d, i (i)}
          <li><span class="day">{date(d.day)}</span> {PLANS[d.plan] ?? 'Logbook row'} <span class="muted">{Math.round(d.minutes)} min</span></li>
        {/each}
      </ul>
    {:else}
      <p class="muted">None yet.</p>
    {/if}
    {#if moved}
      <p class="movement">
        <span class:met={p.movement.strength_days >= p.strength_target}>Strength {p.movement.strength_days} of {p.strength_target}</span>
        · <span class:met={p.movement.active_min >= p.active_min_target}>active {Math.round(p.movement.active_min)} of {p.active_min_target} min</span>
        {#if p.movement.steps_per_day != null}· {Math.round(p.movement.steps_per_day).toLocaleString()} steps a day{/if}
        <span class="muted">· from the Watch</span>
      </p>
    {:else}
      <p class="muted movement">No strength or activity from the Watch yet: add them to the Shortcut.</p>
    {/if}
  </section>

  <section>
    <h2>Today</h2>
    <p class="next">{KINDS[now.kind] ?? now.kind} <span class="muted">{length(now)}</span></p>
    <ul class="why">
      {#each now.why as w (w)}<li>{w}</li>{/each}
    </ul>
    <p class="muted ready">Readiness: <span class={ready.tone}>{ready.mark}</span> {ready.label}</p>
  </section>

  {#if p.power.length}
    <section>
      <h2>Power from last time</h2>
      {#each p.power as w (w.kind)}<p class="power">{w.note}</p>{/each}
    </section>
  {/if}

  <section>
    <h2>Ahead <span class="muted">if you row three times a week; it re-plans after every row</span></h2>
    {#each weeks as week, i (i)}
      <div class="week">
        <h3>{i === 0 ? 'This week' : i === 1 ? 'Next week' : `In ${i} weeks`}</h3>
        <ul class="rows">
          {#each week as a (a.day)}
            <li class:hard={['rate_steps', 'four_by_four', 'rate20_test'].includes(a.kind)}>
              <span class="day">{date(a.day)}</span>
              {KINDS[a.kind] ?? a.kind} <span class="muted">{length(a)}</span>
            </li>
          {/each}
        </ul>
      </div>
    {/each}
    <p class="muted foot">
      Hard rows (rate steps and the 4×4 in turn) start after the comeback ramp, swap for an easy row on an amber or red
      readiness day, and each sets the next one's power. Pick something else on the row screen any time: the plan counts it
      for what it was.
    </p>
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
    margin-left: 0.5rem;
  }
  h3 {
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-2);
    margin: 1rem 0 0.35rem;
  }
  section {
    margin-top: 2.25rem;
    max-width: 44rem;
  }
  .phase {
    margin: 0.4rem 0 0;
    color: var(--ink-2);
  }
  .next {
    margin: 0;
    font-size: 22px;
    font-weight: 600;
  }
  .next .muted {
    font-weight: 400;
  }
  .why {
    margin: 0.5rem 0 0;
    padding-left: 1.1rem;
    color: var(--ink-2);
  }
  .ready {
    margin: 0.6rem 0 0;
  }
  .ok {
    color: var(--ok);
  }
  .warn {
    color: var(--warn);
  }
  .bad {
    color: var(--bad);
  }
  .movement {
    margin: 0.75rem 0 0;
  }
  .movement .met {
    color: var(--ok);
  }
  .power {
    margin: 0 0 0.4rem;
    color: var(--accent);
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .rows li {
    padding: 0.35rem 0;
    border-top: 1px solid var(--rule);
  }
  .rows li.hard {
    color: var(--accent);
  }
  .day {
    display: inline-block;
    width: 8.5rem;
    color: var(--ink-3);
    font-family: var(--font-num);
  }
  .foot {
    margin-top: 1.25rem;
    font-size: 13px;
  }
</style>
