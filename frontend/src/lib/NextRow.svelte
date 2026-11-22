<script>
  // What the next row is set up as, before the first stroke: the planner's
  // proposal (or a workout picked here instead), the cues, and readiness.
  // Re-sent by the server every second, so a pick or new Watch data shows
  // by itself.
  import { pickWorkout } from './api.js';
  import { KINDS, READINESS, minutesLabel, timeLabel } from './format.js';

  let { plan, watchAt } = $props();

  // What can be picked: every workout but the planner's minimum session.
  const CHOICES = ['reference', 'easy', 'steady', 'rate_steps', 'four_by_four', 'rate20_test'];

  const p = $derived(plan.proposal);
  const picked = $derived(plan.kind !== p.kind);
  // The reference row and the test are fixed pieces; their description says
  // so. A pick's length is in its cues.
  const minutes = $derived(picked || ['reference', 'rate20_test'].includes(p.kind) ? '' : minutesLabel(p.minutes));
  const ready = $derived(READINESS[plan.readiness.level] ?? READINESS.unknown);

  let choosing = $state(false);
  let failed = $state(false);
  async function pick(kind) {
    failed = false;
    const res = await pickWorkout(kind).catch(() => null);
    failed = !res?.ok;
    if (!failed) choosing = false;
  }
</script>

<section aria-label="Next row">
  <h2>Next row</h2>
  <p class="kind">{KINDS[plan.kind] ?? plan.kind} <span>{minutes}</span></p>
  {#if picked}
    <p class="instead">
      Instead of the planned {(KINDS[p.kind] ?? p.kind).toLowerCase()}.
      <button class="link" onclick={() => pick(p.kind)}>Back to the plan</button>
    </p>
  {/if}
  {#if plan.cues}<p class="cues">Cues: {plan.cues}</p>{/if}
  {#if plan.progress}<p class="progress">{plan.progress}</p>{/if}
  <p class="pm5">
    Warm up as you like, then press <strong>Start</strong>: it begins 10 seconds later{#if ['rate_steps', 'four_by_four'].includes(plan.kind)}, and this screen times the intervals (the PM5 on Just Row){/if}.
  </p>
  {#if picked}
    <p class="about">{plan.about}</p>
  {:else if p.why.length}
    <ul>
      {#each p.why.slice(0, 3) as w (w)}<li>{w}</li>{/each}
    </ul>
  {/if}
  {#if choosing}
    <div class="choices" role="group" aria-label="Row instead">
      {#each CHOICES as k (k)}
        <button aria-pressed={plan.kind === k} onclick={() => pick(k)}>{KINDS[k]}</button>
      {/each}
    </div>
    <button class="link" onclick={() => (choosing = false)}>Keep it</button>
  {:else}
    <button class="link" onclick={() => (choosing = true)}>Row something else…</button>
  {/if}
  {#if failed}<p class="failed" role="alert">Not changed: is a row under way?</p>{/if}

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
  .progress {
    margin: 0.4rem 0 0;
    font-size: 16px;
    color: var(--accent);
  }
  .instead,
  .about {
    margin: 0.4rem 0 0;
    font-size: 16px;
    color: var(--ink-2);
  }
  .choices {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin: 1rem 0 0.5rem;
  }
  .choices button {
    padding: 0.45rem 0.9rem;
    font-size: 16px;
  }
  .choices button[aria-pressed='true'] {
    color: var(--ink);
    border-color: var(--accent);
    background: var(--accent-wash);
  }
  .link {
    margin-top: 0.75rem;
    border-color: transparent;
    color: var(--ink-3);
    padding-left: 0;
    font-size: 16px;
  }
  .instead .link {
    margin: 0 0 0 0.4rem;
  }
  .failed {
    margin: 0.4rem 0 0;
    color: var(--bad);
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
