<script>
  // Time in each heart-rate zone as one whole split into parts (tall segmented
  // bar). Zones are ordered, so they take one hue stepping in lightness.
  import { clock } from './format.js';

  let { seconds = [], zones = [] } = $props();

  // Label ink inside each block, by measured contrast against the fill.
  const INK = ['#ffffff', '#ffffff', '#0c131b', '#0c131b', '#0c131b', '#0c131b'];
  let width = $state(600);
  let focus = $state(null);
  // Click pins a zone's readout so it stays after the pointer leaves.
  let pinned = $state(null);
  const shown = $derived(focus ?? pinned);

  const total = $derived(seconds.reduce((a, b) => a + b, 0));
  const parts = $derived(
    seconds
      .map((s, z) => ({ z, s, share: total ? s / total : 0 }))
      .filter((p) => p.s >= 1),
  );
  const range = (z) => {
    if (z === 0) return zones[0] ? `under ${Math.round(zones[0].lo)}` : '';
    const b = zones.find((x) => x.zone === z);
    return b ? `${Math.round(b.lo)}–${Math.round(b.hi)}` : '';
  };
  const label = (p) => `Z${p.z} ${clock(p.s)}`;
  // Roughly 7.5px a character at 12px, plus padding: print only if it fits.
  const fits = (p) => p.share * width >= label(p).length * 7.5 + 14;
</script>

{#if total > 0}
  <div class="zones">
    <div class="bar" bind:clientWidth={width} role="group" aria-label="Time in heart-rate zones">
      {#each parts as p (p.z)}
        <button
          type="button"
          class="part"
          style:flex-grow={p.s}
          style:background="var(--z{p.z})"
          style:color={INK[p.z]}
          aria-label="Zone {p.z}: {clock(p.s)}, {Math.round(p.share * 100)}%"
          aria-pressed={pinned?.z === p.z}
          onpointerenter={() => (focus = p)}
          onpointerleave={() => (focus = null)}
          onfocus={() => (focus = p)}
          onblur={() => (focus = null)}
          onclick={() => (pinned = pinned?.z === p.z ? null : p)}
        >
          {#if fits(p)}<span>{label(p)}</span>{/if}
        </button>
      {/each}
    </div>
    <div class="legend">
      {#each parts as p (p.z)}
        <span class="item"><span class="swatch" style:background="var(--z{p.z})"></span>Z{p.z}<span class="muted">{range(p.z)}</span></span>
      {/each}
      <span class="readout" aria-live="polite">
        {#if shown}<strong class="num">{clock(shown.s)}</strong> in Z{shown.z} · {Math.round(shown.share * 100)}%{/if}
      </span>
    </div>
  </div>
{/if}

<style>
  .bar {
    display: flex;
    gap: 2px;
    height: 28px;
  }
  /* Width is exactly the share: no basis, no padding (the label is inset instead). */
  .part {
    flex-basis: 0;
    min-width: 2px;
    border: 0;
    padding: 0;
    border-radius: 4px;
    text-align: left;
    font: 600 12px/28px var(--font-num);
    white-space: nowrap;
    cursor: default;
  }
  .part span {
    margin-left: 7px;
  }
  .part:hover,
  .part[aria-pressed='true'] {
    filter: brightness(1.12);
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem 1.1rem;
    margin-top: 0.5rem;
    font-size: 13px;
    color: var(--ink-2);
  }
  .item {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
  }
  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 2px;
  }
  .readout {
    margin-left: auto;
    color: var(--ink-2);
  }
  .readout strong {
    color: var(--ink);
  }
</style>
