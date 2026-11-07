<script>
  // What to row today, from the planner: the session and why. The row screen
  // (/) is set up for it and records from the first stroke.
  import { getToday } from './api.js';
  import { KINDS, minutesLabel } from './format.js';

  let t = $state(null);
  getToday()
    .then((x) => (t = x))
    .catch(() => (t = null));

  const p = $derived(t?.proposal);
  const minutes = $derived(p ? minutesLabel(p.minutes) : '');
</script>

{#if p}
  <section aria-label="Today">
    <h2>Today</h2>
    <p class="kind"><strong>{KINDS[p.kind] ?? p.kind}</strong> <span class="muted">{minutes}</span></p>
    {#if p.why.length}
      <ul>
        {#each p.why.slice(0, 2) as w (w)}<li>{w}</li>{/each}
      </ul>
    {/if}
    {#if t.pm5_setup}<p class="setup">On the PM5: {t.pm5_setup}</p>{/if}
  </section>
{/if}

<style>
  section {
    margin: 0 1rem 1.75rem 0;
    font-size: 13px;
  }
  h2 {
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-3);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 0 0 0.4rem;
  }
  .kind {
    margin: 0;
    font-size: 15px;
  }
  ul {
    margin: 0.35rem 0 0;
    padding-left: 1rem;
    color: var(--ink-2);
  }
  .setup {
    margin: 0.4rem 0 0;
    color: var(--ink-2);
  }
</style>
