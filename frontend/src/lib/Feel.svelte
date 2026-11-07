<script>
  // How the row felt, in the rower's words: the numbers can't say whether a
  // high heart rate was a hot room, a bad night or real fatigue. Saved with
  // the session; re-analysing it keeps them.
  import { setFeel } from './api.js';

  let { id, effort = null, comment = null } = $props();

  const SCALE = [
    [1, 'very easy'],
    [2, 'easy'],
    [3, 'easy'],
    [4, 'moderate'],
    [5, 'moderate'],
    [6, 'somewhat hard'],
    [7, 'hard'],
    [8, 'hard'],
    [9, 'very hard'],
    [10, 'all out'],
  ];

  let chosen = $state(null);
  let text = $state('');
  let savedText = $state('');
  let status = $state(null);

  // A different session: start from what it has. Only the props are read
  // here, so typing doesn't re-run it.
  $effect(() => {
    void id;
    const start = comment ?? '';
    chosen = effort;
    text = start;
    savedText = start;
    status = null;
  });

  async function save(nextEffort = chosen) {
    status = 'saving';
    const res = await setFeel(id, nextEffort, text.trim() || null).catch(() => null);
    if (res?.ok) {
      chosen = nextEffort;
      savedText = text;
      status = 'saved';
    } else {
      status = 'failed';
    }
  }

  const pick = (n) => save(chosen === n ? null : n);
  const dirty = $derived(text !== savedText);
</script>

<section class="feel" aria-labelledby="feel-title">
  <h2 id="feel-title">How did it feel?</h2>
  <div class="scale" role="group" aria-label="Effort, 1 very easy to 10 all out">
    {#each SCALE as [n, word] (n)}
      <button class="num" aria-pressed={chosen === n} title={word} onclick={() => pick(n)}>{n}</button>
    {/each}
    <span class="word">{chosen ? SCALE[chosen - 1][1] : '1 very easy · 10 all out'}</span>
  </div>
  <textarea
    bind:value={text}
    rows="2"
    maxlength="1000"
    placeholder="Anything worth knowing: sleep, a sore back, a hot room, a stop to fix the strap…"
    aria-label="Comment"
    onblur={() => dirty && save()}
  ></textarea>
  <div class="foot">
    {#if dirty}<button onclick={() => save()}>Save</button>{/if}
    <span class="status" aria-live="polite">
      {status === 'saving' ? 'Saving…' : status === 'saved' ? 'Saved' : status === 'failed' ? 'Not saved: is Row Assist running?' : ''}
    </span>
  </div>
</section>

<style>
  .feel {
    margin-top: 1.5rem;
    max-width: 44rem;
  }
  h2 {
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-3);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 0 0 0.6rem;
  }
  .scale {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem;
  }
  .scale button {
    min-width: 2.4rem;
    padding: 0.35rem 0;
  }
  .scale button[aria-pressed='true'] {
    color: var(--ink);
    border-color: var(--accent);
    background: var(--accent-wash);
  }
  .word {
    margin-left: 0.6rem;
    color: var(--ink-2);
    font-size: 14px;
  }
  textarea {
    display: block;
    width: 100%;
    box-sizing: border-box;
    margin-top: 0.75rem;
    font: inherit;
    font-size: 15px;
    color: var(--ink);
    background: var(--bg);
    border: 1px solid var(--rule);
    border-radius: 4px;
    padding: 0.5rem 0.65rem;
    resize: vertical;
  }
  textarea:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    min-height: 2rem;
    margin-top: 0.4rem;
  }
  .status {
    font-size: 13px;
    color: var(--ink-3);
  }
</style>
