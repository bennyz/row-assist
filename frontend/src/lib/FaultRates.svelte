<script>
  // Share of strokes showing each fault: one bar per fault, value at the tip.
  import { FAULTS } from './format.js';

  let { rates = [], camera = false } = $props();

  const rows = $derived(
    rates.map((r) => ({ ...r, share: r.of ? r.hits / r.of : 0, label: FAULTS[r.fault]?.label ?? r.fault })),
  );
</script>

<div class="rates">
  {#each rows as r (r.fault)}
    <div class="row">
      <span class="label">{r.label}</span>
      <span class="track">
        {#if r.share > 0}<span class="bar" style:width="{Math.max(r.share * 85, 1)}%"></span>{/if}
        <span class="value num tab">{Math.round(r.share * 100)}%</span>
      </span>
      <span class="of muted tab">of {r.of}</span>
    </div>
  {/each}
  {#if !camera}
    <p class="muted note">Posture faults need the camera: <code>rowctl record --camera</code>.</p>
  {/if}
</div>

<style>
  .rates {
    display: grid;
    gap: 0.45rem;
    max-width: 40rem;
  }
  .row {
    display: grid;
    grid-template-columns: 11rem 1fr 4.5rem;
    align-items: center;
    gap: 0.75rem;
    font-size: 14px;
  }
  .label {
    color: var(--ink-2);
  }
  .track {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .bar {
    height: 10px;
    background: var(--ink-2);
    border-radius: 0 4px 4px 0;
  }
  .value {
    color: var(--ink);
  }
  .of {
    font-size: 12px;
    text-align: right;
  }
  .note {
    font-size: 13px;
    margin: 0.4rem 0 0;
  }
  code {
    font-family: var(--font-num);
  }
</style>
