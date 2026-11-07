<script>
  // A big number with its target band under it, read from the erg: the
  // band is shaded, the marker is where you are, and a word says which way
  // to go when you're outside it. `band` is [low, high]; either may be null
  // (a ceiling only, like heart rate).
  let { value, label, band = null, digits = 0, extra = null, hints = {} } = $props();

  const lo = $derived(band?.[0] ?? null);
  const hi = $derived(band?.[1] ?? null);
  // The scale: the band with room either side, so a marker outside shows.
  const scale = $derived.by(() => {
    if (lo == null && hi == null) return null;
    const a = lo ?? hi * 0.7;
    const b = hi ?? lo * 1.3;
    const pad = Math.max((b - a) * 0.8, b * 0.06);
    return [a - pad, b + pad];
  });
  const at = (v) => Math.min(100, Math.max(0, ((v - scale[0]) / (scale[1] - scale[0])) * 100));
  const where = $derived(
    value == null || !scale ? null : lo != null && value < lo ? 'low' : hi != null && value > hi ? 'high' : 'ok',
  );
  const aim = $derived(
    lo != null && hi != null ? `aim ${lo}–${hi}` : hi != null ? `under ${hi}` : lo != null ? `over ${lo}` : '',
  );
  const hint = $derived(where === 'low' ? hints.low : where === 'high' ? hints.high : null);
</script>

<div class="gauge" data-where={where}>
  <div class="value num">{value == null ? '–' : value.toFixed(digits)}{#if extra}<span class="extra">{extra}</span>{/if}</div>
  <div class="label">
    {label}
    {#if aim}<span class="aim">{aim}</span>{/if}
  </div>
  {#if scale}
    <div class="track" aria-hidden="true">
      <div class="band" style:left="{at(lo ?? scale[0])}%" style:right="{100 - at(hi ?? scale[1])}%"></div>
      {#if value != null}<div class="mark" style:left="{at(value)}%"></div>{/if}
    </div>
    <p class="hint" aria-live="polite">
      {#if hint}<span aria-hidden="true">{where === 'low' ? '↑' : '↓'}</span> {hint}{:else if where === 'ok'}<span class="ok">✓ on target</span>{/if}
    </p>
  {/if}
</div>

<style>
  .gauge {
    min-width: 11rem;
  }
  .value {
    font-size: 72px;
    font-weight: 600;
    line-height: 1;
  }
  .extra {
    font-size: 36px;
    color: var(--ink-2);
    margin-left: 0.6rem;
  }
  .label {
    margin-top: 0.35rem;
    color: var(--ink-3);
    font-size: 16px;
  }
  .aim {
    margin-left: 0.5rem;
    color: var(--ink-2);
  }
  .track {
    position: relative;
    height: 10px;
    margin-top: 0.6rem;
    border-radius: 5px;
    background: var(--rule);
  }
  .band {
    position: absolute;
    top: 0;
    bottom: 0;
    border-radius: 5px;
    background: var(--accent-wash);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .mark {
    position: absolute;
    top: -5px;
    width: 6px;
    height: 20px;
    margin-left: -3px;
    border-radius: 3px;
    background: var(--ink);
    box-shadow: 0 0 0 2px var(--surface);
  }
  [data-where='low'] .mark,
  [data-where='high'] .mark {
    background: var(--warn);
  }
  .hint {
    min-height: 1.5rem;
    margin: 0.4rem 0 0;
    font-size: 20px;
    color: var(--ink);
  }
  .hint span[aria-hidden] {
    color: var(--warn);
  }
  .ok {
    color: var(--ink-3);
    font-size: 16px;
  }
</style>
