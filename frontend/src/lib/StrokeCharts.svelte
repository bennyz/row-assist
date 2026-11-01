<script>
  // Power, rate and heart rate per stroke as small multiples on one time axis
  // (never one chart with two y-scales). One crosshair spans all three; click
  // or arrow keys pick the stroke shown in the force-curve panel.
  import { clock, fixed, linePath, nearest, niceTicks } from './format.js';

  let { strokes, zones = [], selected = $bindable() } = $props();

  const PANELS = [
    { key: 'watts', label: 'Power', unit: 'W', digits: 0 },
    { key: 'rate', label: 'Rate', unit: 'spm', digits: 1 },
    { key: 'hr', label: 'Heart rate', unit: 'bpm', digits: 0 },
  ];
  const PLOT_H = 92;
  const TITLE_H = 28;
  const GAP = 18;
  const M = { l: 44, r: 44, b: 26 };
  const STEP = TITLE_H + PLOT_H + GAP;

  let width = $state(720);
  let hover = $state(null);
  let pointer = $state({ x: 0, y: 0 });

  const height = PANELS.length * STEP - GAP + M.b;
  const t0 = $derived(strokes[0]?.t_s ?? 0);
  const t1 = $derived(Math.max(strokes.at(-1)?.t_s ?? 1, t0 + 1));
  const x = (t) => M.l + ((t - t0) / (t1 - t0)) * (width - M.l - M.r);

  const xTicks = $derived(niceTicks(t0 / 60, t1 / 60, Math.max(2, Math.floor(width / 110))).ticks.filter(
    (m) => m * 60 >= t0 - 1 && m * 60 <= t1 + 1,
  ));

  const panels = $derived(
    PANELS.map((p, i) => {
      const vals = strokes.map((s) => s[p.key]).filter((v) => v != null);
      let lo = Math.min(...vals);
      let hi = Math.max(...vals);
      if (!vals.length) [lo, hi] = [0, 1];
      const pad = Math.max((hi - lo) * 0.08, p.key === 'rate' ? 0.5 : 2);
      const scale = niceTicks(lo - pad, hi + pad, 3);
      const top = i * STEP + TITLE_H;
      const y = (v) => top + PLOT_H - ((v - scale.lo) / (scale.hi - scale.lo)) * PLOT_H;
      const bands =
        p.key === 'hr'
          ? zones
              .map((z) => ({ label: `Z${z.zone}`, v: z.lo }))
              .filter((b) => b.v > scale.lo && b.v < scale.hi)
          : [];
      return {
        ...p,
        top,
        y,
        scale,
        bands,
        path: linePath(strokes.map((s) => [x(s.t_s), s[p.key] == null ? null : y(s[p.key])])),
      };
    }),
  );

  const selIdx = $derived(strokes.findIndex((s) => s.index === selected));

  function move(e) {
    const r = e.currentTarget.getBoundingClientRect();
    const px = e.clientX - r.left;
    const t = t0 + ((px - M.l) / (width - M.l - M.r)) * (t1 - t0);
    hover = nearest(strokes, 't_s', t);
    pointer = { x: px, y: e.clientY - r.top };
  }

  function key(e) {
    const step = { ArrowLeft: -1, ArrowRight: 1 }[e.key];
    if (!step || !strokes.length) return;
    e.preventDefault();
    const i = Math.min(strokes.length - 1, Math.max(0, (selIdx < 0 ? 0 : selIdx) + step));
    selected = strokes[i].index;
  }

  const hs = $derived(hover == null ? null : strokes[hover]);
</script>

<div class="charts" bind:clientWidth={width}>
  <svg {width} {height} role="img" aria-label="Power, rate and heart rate for each stroke over the session">
    {#each panels as p (p.key)}
      <text class="title" x={0} y={p.top - 12}>{p.label}<tspan class="unit" dx="6">{p.unit}</tspan></text>
      {#each p.scale.ticks as v (v)}
        <line class="grid" x1={M.l} x2={width - M.r} y1={p.y(v)} y2={p.y(v)} />
        <text class="tick tab" x={M.l - 8} y={p.y(v) + 4} text-anchor="end">{v}</text>
      {/each}
      {#each p.bands as b (b.label)}
        <line class="band" x1={M.l} x2={width - M.r} y1={p.y(b.v)} y2={p.y(b.v)} />
        <text class="tick" x={width - M.r + 6} y={p.y(b.v) + 4}>{b.label}</text>
      {/each}
      <path class="series" d={p.path} />
    {/each}

    {#each xTicks as m (m)}
      <text class="tick tab" x={x(m * 60)} y={height - 6} text-anchor="middle">{m}</text>
    {/each}
    <text class="tick" x={width - M.r + 6} y={height - 6}>min</text>

    {#if selIdx >= 0}
      {@const s = strokes[selIdx]}
      {#each panels as p (p.key)}
        <line class="selected" x1={x(s.t_s)} x2={x(s.t_s)} y1={p.top} y2={p.top + PLOT_H} />
        {#if s[p.key] != null}
          <circle class="dot sel" cx={x(s.t_s)} cy={p.y(s[p.key])} r="4" />
        {/if}
      {/each}
    {/if}

    {#if hs}
      {#each panels as p (p.key)}
        <line class="cross" x1={x(hs.t_s)} x2={x(hs.t_s)} y1={p.top} y2={p.top + PLOT_H} />
        {#if hs[p.key] != null}
          <circle class="dot" cx={x(hs.t_s)} cy={p.y(hs[p.key])} r="4" />
        {/if}
      {/each}
    {/if}

    <rect
      class="hit"
      x={M.l}
      y={0}
      width={Math.max(0, width - M.l - M.r)}
      height={height - M.b}
      role="slider"
      tabindex="0"
      aria-label="Stroke: arrow keys to step, click to select"
      aria-valuemin={strokes[0]?.index ?? 0}
      aria-valuemax={strokes.at(-1)?.index ?? 0}
      aria-valuenow={selected ?? 0}
      onpointermove={move}
      onpointerleave={() => (hover = null)}
      onclick={() => hs && (selected = hs.index)}
      onkeydown={key}
    />
  </svg>

  {#if hs}
    <div
      class="tooltip"
      style:left="{Math.min(pointer.x + 14, width - 150)}px"
      style:top="{Math.max(0, pointer.y - 70)}px"
    >
      <div class="row muted">Stroke {hs.index + 1} · {clock(hs.t_s)}</div>
      {#each PANELS as p (p.key)}
        <div class="row"><strong>{fixed(hs[p.key], p.digits)}</strong><span class="muted">{p.unit}</span></div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .charts {
    position: relative;
    width: 100%;
  }
  svg {
    display: block;
    overflow: visible;
  }
  .title {
    fill: var(--ink-2);
    font-size: 13px;
    font-weight: 500;
  }
  .unit {
    fill: var(--ink-3);
    font-weight: 400;
  }
  .tick {
    fill: var(--ink-3);
    font-size: 11px;
  }
  .grid {
    stroke: var(--rule);
    stroke-width: 1;
  }
  .band {
    stroke: var(--z1);
    stroke-opacity: 0.45;
    stroke-width: 1;
  }
  .series {
    fill: none;
    stroke: var(--ink-2);
    stroke-width: 2;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .cross {
    stroke: var(--ink-3);
    stroke-width: 1;
  }
  .selected {
    stroke: var(--accent);
    stroke-width: 1;
  }
  .dot {
    fill: var(--ink);
    stroke: var(--surface);
    stroke-width: 2;
  }
  .dot.sel {
    fill: var(--accent);
  }
  .hit {
    fill: transparent;
    cursor: crosshair;
    outline: none;
  }
  .hit:focus-visible {
    stroke: var(--accent);
    stroke-width: 1;
  }
</style>
