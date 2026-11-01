<script>
  // What to row today, from the planner: the session, why, and the command.
  import { getToday } from './api.js';

  const KINDS = {
    reference: 'Reference row',
    easy: 'Easy row',
    steady: 'Steady row',
    rate_steps: 'Rate steps',
    rate20_test: 'Rate-20 test',
    minimum: 'Minimum session',
  };

  let t = $state(null);
  let copied = $state(false);
  getToday()
    .then((x) => (t = x))
    .catch(() => (t = null));

  const p = $derived(t?.proposal);
  const minutes = $derived(p ? (p.minutes[0] === p.minutes[1] ? `${p.minutes[0]} min` : `${p.minutes[0]}–${p.minutes[1]} min`) : '');

  async function copy() {
    try {
      await navigator.clipboard.writeText(t.command);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      // No clipboard access (e.g. plain http on another machine): select it instead.
      const range = document.createRange();
      range.selectNodeContents(document.getElementById('today-cmd'));
      getSelection().removeAllRanges();
      getSelection().addRange(range);
    }
  }
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
    <div class="cmd">
      <!-- Break only between words: "--speak" mustn't split at its hyphens. -->
      <code id="today-cmd">{#each t.command.split(' ') as tok, i (i)}<span class="tok">{tok}</span>{' '}{/each}</code>
      <button onclick={copy}>{copied ? 'Copied' : 'Copy'}</button>
    </div>
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
  .cmd {
    display: grid;
    gap: 0.35rem;
    margin-top: 0.6rem;
  }
  code {
    display: block;
    font: 12px/1.4 var(--font-num);
    color: var(--ink);
    background: var(--surface);
    border: 1px solid var(--rule);
    border-radius: 4px;
    padding: 0.4rem 0.5rem;
  }
  .tok {
    white-space: nowrap;
  }
  button {
    justify-self: start;
    font-size: 12px;
  }
</style>
