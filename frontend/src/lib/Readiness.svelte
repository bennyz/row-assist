<script>
  // Today's readiness from the Watch: a level with its reasons, never colour alone.
  import { getHealth } from './api.js';
  import { fixed } from './format.js';

  const LEVELS = {
    green: { mark: '✓', label: 'Ready', tone: 'ok' },
    amber: { mark: '▲', label: 'Go easy', tone: 'warn' },
    red: { mark: '■', label: 'Recover', tone: 'bad' },
    unknown: { mark: '?', label: 'No baseline yet', tone: 'muted' },
  };
  const ADVICE = {
    green: 'Row as planned.',
    amber: 'Easy only: no intensity today.',
    red: 'Minimum session: 15–20 min easy, or rest.',
    unknown: 'Needs two weeks of Watch data.',
  };

  let health = $state(null);
  getHealth()
    .then((h) => (health = h))
    .catch(() => (health = null));

  const r = $derived(health?.readiness);
  const level = $derived(LEVELS[r?.level] ?? LEVELS.unknown);
</script>

{#if health?.days?.length}
  <section aria-label="Readiness">
    <div class="level {level.tone}">
      <span class="mark" aria-hidden="true">{level.mark}</span>
      <strong>{level.label}</strong>
    </div>
    <p class="advice">{ADVICE[r.level]}</p>
    {#if r.reasons.length}
      <ul>
        {#each r.reasons as reason (reason)}<li>{reason}</li>{/each}
      </ul>
    {/if}
    <p class="facts muted">
      HRV {r.hrv_week_ln == null ? '–' : fixed(Math.exp(r.hrv_week_ln))} ms · rest HR {fixed(r.rhr_week)} · sleep
      {r.sleep_last_h == null ? '–' : `${fixed(r.sleep_last_h, 1)} h`}
    </p>
  </section>
{/if}

<style>
  section {
    margin: 0 1rem 1.75rem 0;
    padding: 0.75rem 0.75rem 0.6rem;
    border: 1px solid var(--rule);
    border-radius: 4px;
    font-size: 13px;
  }
  .level {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: 15px;
  }
  .ok .mark {
    color: var(--ok);
  }
  .warn .mark {
    color: var(--warn);
  }
  .bad .mark {
    color: var(--bad);
  }
  .muted .mark {
    color: var(--ink-3);
  }
  .advice {
    margin: 0.3rem 0 0;
    color: var(--ink);
  }
  ul {
    margin: 0.4rem 0 0;
    padding-left: 1rem;
    color: var(--ink-2);
  }
  .facts {
    margin: 0.5rem 0 0;
  }
</style>
