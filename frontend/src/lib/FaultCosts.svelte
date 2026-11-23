<script>
  // What each drive fault went with in work per stroke: the third of strokes
  // that showed it most against the third that showed it least, each
  // stroke's work against the strokes around it (analysis::cost). For a clear
  // difference, both thirds' mean force curves show where in the drive they
  // part. What went together, not proof of cause.
  import { fixed, linePath } from './format.js';

  let { costs = [] } = $props();

  // The measure behind each fault, named for what it measures (a rower can
  // be on the good side of a fault's line and still differ along it), and how
  // a third rowed it.
  const MEASURES = {
    short_compression: {
      name: 'How far forward at the catch',
      of: (v) =>
        Math.abs(v) < 0.5 ? 'shins vertical' : v > 0 ? `shins ${fixed(v)}° short of vertical` : `shins ${fixed(-v)}° past vertical`,
    },
    shooting_slide: {
      name: 'The back holding as the legs push',
      of: (v) => (v < 0.05 ? 'back still' : `back tipping ${fixed(v, 1)}° forward`),
    },
    early_body: { name: 'When the body swings', of: (v) => `body swinging from ${fixed(v * 100)}% of the drive` },
    early_arms: { name: 'When the arms bend', of: (v) => `arms bent ${fixed(v)}° from the catch while the legs pushed` },
  };

  const H = 120;
  const M = { l: 8, r: 8, t: 8, b: 20 };
  let width = $state(420);
  const w = $derived(Math.min(width, 460));

  // The worse third against the better, in words.
  function effect(c) {
    const m = MEASURES[c.fault];
    const [worse, better] = [m.of(c.worst), m.of(c.best)];
    if (!c.clear) return { worse, better, text: 'no clear difference in work', tone: 'muted' };
    const watts = fixed(Math.abs((c.watts * c.work_pct) / 100));
    const more = c.work_pct < 0 ? 'less' : 'more';
    return {
      worse,
      better,
      text: `${fixed(Math.abs(c.work_pct), 1)}% ${more} work per stroke (≈ ${watts} W)`,
      tone: c.work_pct < 0 ? 'bad' : 'ok',
    };
  }

  function paths(c) {
    const top = Math.max(1, ...c.worst_curve, ...c.best_curve) * 1.05;
    const n = c.worst_curve.length;
    const x = (i) => M.l + (i / (n - 1)) * (w - M.l - M.r);
    const y = (v) => M.t + (H - M.t - M.b) * (1 - v / top);
    const line = (curve) => linePath(curve.map((v, i) => [x(i), y(v)]));
    return { worst: line(c.worst_curve), best: line(c.best_curve), x };
  }
</script>

<div class="costs" bind:clientWidth={width}>
  {#each costs as c (c.fault)}
    {@const e = effect(c)}
    <div class="cost">
      <p class="head">
        <span class="label">{MEASURES[c.fault].name}</span>
        <span class={e.tone}>{e.text}</span>
      </p>
      <p class="thirds muted">
        The {c.strokes} strokes with {e.worse}, against the {c.strokes} with {e.better}.
      </p>
      {#if c.clear && c.worst_curve.length > 1}
        {@const p = paths(c)}
        <svg width={w} height={H} role="img" aria-label="Mean force through the drive of both thirds">
          <line class="base" x1={M.l} x2={w - M.r} y1={H - M.b} y2={H - M.b} />
          {#each [0, 50, 100] as at (at)}
            <text class="tick" x={p.x(((c.worst_curve.length - 1) * at) / 100)} y={H - 5} text-anchor="middle">{at}%</text>
          {/each}
          <path class="best" d={p.best} />
          <path class="worst" d={p.worst} />
        </svg>
        <p class="legend">
          <span><span class="key worst"></span>{e.worse}</span>
          <span><span class="key best"></span>{e.better}</span>
          <span class="muted">force through the drive</span>
        </p>
      {/if}
    </div>
  {/each}
  <p class="note muted">
    Each stroke's work is taken against the strokes around it, so intervals and tiring don't count. It shows what went
    together, not what caused what.
  </p>
</div>

<style>
  .costs {
    display: grid;
    gap: 1.1rem;
    max-width: 46rem;
  }
  .cost p {
    margin: 0;
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem 0.75rem;
    font-size: 14px;
  }
  .label {
    min-width: 11rem;
    color: var(--ink);
    font-weight: 500;
  }
  .bad {
    color: var(--bad);
  }
  .ok {
    color: var(--ok);
  }
  .thirds {
    font-size: 13px;
    margin-top: 0.2rem !important;
  }
  svg {
    display: block;
    margin-top: 0.5rem;
    overflow: visible;
  }
  .base {
    stroke: var(--rule);
  }
  .tick {
    fill: var(--ink-3);
    font-size: 11px;
  }
  path {
    fill: none;
    stroke-width: 2;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  /* Orange against grey: the two must part at a glance. */
  path.best {
    stroke: var(--ink-2);
  }
  path.worst {
    stroke: var(--accent);
  }
  .legend {
    display: flex;
    gap: 1rem;
    font-size: 12px;
    color: var(--ink-2);
  }
  .legend > span {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
  }
  .key {
    display: inline-block;
    width: 14px;
    height: 2px;
  }
  .key.best {
    background: var(--ink-2);
  }
  .key.worst {
    background: var(--accent);
  }
  .note {
    font-size: 12px;
  }
</style>
