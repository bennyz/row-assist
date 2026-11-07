<script>
  // What the next row is set up as, before the first stroke: the planner's
  // proposal, the cues, and readiness. Re-sent by the server when Watch data
  // arrives, so it updates by itself.
  import { KINDS, READINESS, minutesLabel, timeLabel } from './format.js';

  let { plan, watchAt } = $props();

  const p = $derived(plan.proposal);
  // The reference row and the test are fixed pieces; their description says so.
  const minutes = $derived(['reference', 'rate20_test'].includes(p.kind) ? '' : minutesLabel(p.minutes));
  const ready = $derived(READINESS[plan.readiness.level] ?? READINESS.unknown);
</script>

<section aria-label="Next row">
  <h2>Next row</h2>
  <p class="kind">{KINDS[p.kind] ?? p.kind} <span>{minutes}</span></p>
  {#if plan.cues}<p class="cues">Cues: {plan.cues}</p>{/if}
  {#if plan.pm5_setup}<p class="pm5">Set on the PM5: {plan.pm5_setup}</p>{/if}
  {#if p.why.length}
    <ul>
      {#each p.why.slice(0, 3) as w (w)}<li>{w}</li>{/each}
    </ul>
  {/if}

  <h2>Readiness</h2>
  <p class="level {ready.tone}"><span aria-hidden="true">{ready.mark}</span> {ready.label}</p>
  {#each plan.readiness.reasons as reason (reason)}<p class="reason">{reason}</p>{/each}
  <p class="watch">
    {#if watchAt}
      Watch data in at {timeLabel(watchAt)}.
    {:else}
      Tap <strong>Row Assist health</strong> on your phone to bring in last night’s Watch data.
    {/if}
  </p>
</section>

<style>
  h2 {
    font-size: 15px;
    font-weight: 500;
    color: var(--ink-3);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 0 0 0.6rem;
  }
  h2:not(:first-child) {
    margin-top: 1.75rem;
  }
  .kind {
    margin: 0;
    font-size: 30px;
    font-weight: 600;
    color: var(--ink);
  }
  .kind span {
    font-weight: 400;
    color: var(--ink-2);
  }
  .cues,
  .pm5 {
    margin: 0.5rem 0 0;
    font-size: 19px;
    color: var(--ink);
  }
  ul {
    margin: 0.75rem 0 0;
    padding-left: 1.1rem;
    font-size: 16px;
    color: var(--ink-2);
  }
  li + li {
    margin-top: 0.3rem;
  }
  .level {
    margin: 0;
    font-size: 22px;
    color: var(--ink);
  }
  .ok span {
    color: var(--ok);
  }
  .warn span {
    color: var(--warn);
  }
  .bad span {
    color: var(--bad);
  }
  .muted span {
    color: var(--ink-3);
  }
  .reason {
    margin: 0.3rem 0 0;
    font-size: 16px;
    color: var(--ink-2);
  }
  .watch {
    margin: 0.6rem 0 0;
    font-size: 16px;
    color: var(--ink-3);
  }
  .watch strong {
    color: var(--ink-2);
    font-weight: 500;
  }
</style>
