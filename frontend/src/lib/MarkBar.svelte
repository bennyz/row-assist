<script>
  // Marking what's rowed on purpose, so the fault detectors can be checked
  // against it (`rowctl check`): pick Clean or a fault before a block of
  // strokes, and the recording knows which strokes were meant to be which.
  // Before the first stroke the mark waits and the row starts with it.
  import { markRow } from './api.js';
  import { FAULTS } from './format.js';

  let { mark } = $props();

  const CHOICES = [
    { key: 'clean', fault: null, label: 'Clean' },
    ...Object.entries(FAULTS).map(([fault, f]) => ({ key: fault, fault, label: f.label })),
  ];

  let opened = $state(false);
  let failed = $state(false);
  // Open while a mark holds: it shows what this block is.
  const open = $derived(opened || mark != null);
  const current = $derived(mark == null ? null : (mark.fault ?? 'clean'));

  async function pick(fault) {
    failed = false;
    const res = await markRow(fault).catch(() => null);
    failed = !res?.ok;
  }
</script>

<div class="marks">
  {#if open}
    <span class="what" id="marks-label">Rowing on purpose:</span>
    <div class="choices" role="group" aria-labelledby="marks-label">
      {#each CHOICES as c (c.key)}
        <button aria-pressed={current === c.key} onclick={() => pick(c.fault)}>{c.label}</button>
      {/each}
    </div>
    {#if mark == null}
      <button class="close" onclick={() => (opened = false)}>Hide</button>
    {/if}
    {#if failed}<span class="failed" role="alert">Not marked: is Row Assist running?</span>{/if}
  {:else}
    <button class="open" onclick={() => (opened = true)}>Mark faults on purpose…</button>
  {/if}
</div>

<style>
  .marks {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.6rem 1rem;
    font-size: 17px;
  }
  .what {
    color: var(--ink-3);
  }
  .choices {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .choices button {
    padding: 0.45rem 0.9rem;
  }
  .choices button[aria-pressed='true'] {
    color: var(--ink);
    border-color: var(--accent);
    background: var(--accent-wash);
  }
  .open,
  .close {
    border-color: transparent;
    color: var(--ink-3);
    padding-left: 0;
  }
  .failed {
    color: var(--bad);
  }
</style>
