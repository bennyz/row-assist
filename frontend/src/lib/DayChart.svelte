<script>
  // A measure over calendar days: lines and/or dots, optional reference
  // lines, one y-axis. Hover finds the nearest day across all series.
  import { fixed, linePath, niceTicks } from './format.js';

  let {
    series = [],
    x0,
    x1,
    height = 150,
    unit = '',
    digits = 0,
    zero = false,
    refs = [],
  } = $props();

  const M = { l: 48, r: 72, t: 24, b: 26 };
  let width = $state(720);
  let hover = $state(null);
  let pointer = $state({ x: 0, y: 0 });

  const all = $derived(series.flatMap((s) => s.points.map((p) => p.value)).concat(refs.map((r) => r.value)));
  const scale = $derived.by(() => {
    if (!all.length) return niceTicks(0, 1, 3);
    let lo = Math.min(...all);
    let hi = Math.max(...all);
    if (zero) lo = Math.min(0, lo);
    const pad = (hi - lo) * 0.1 || Math.abs(hi) * 0.1 || 1;
    return niceTicks(zero ? lo : lo - pad, hi + pad, 3);
  });
  const x = (d) => M.l + ((d - x0) / Math.max(1, x1 - x0)) * (width - M.l - M.r);
  const y = (v) => M.t + (height - M.t - M.b) * (1 - (v - scale.lo) / (scale.hi - scale.lo));

  // Month ticks, thinned to fit.
  const months = $derived.by(() => {
    const out = [];
    const d = new Date(x0 * 86400000);
    d.setUTCDate(1);
    d.setUTCMonth(d.getUTCMonth() + 1);
    while (d.getTime() / 86400000 <= x1) {
      out.push({ day: d.getTime() / 86400000, month: d.getUTCMonth(), year: d.getUTCFullYear() });
      d.setUTCMonth(d.getUTCMonth() + 1);
    }
    const every = Math.max(1, Math.ceil(out.length / Math.max(2, Math.floor((width - M.l - M.r) / 60))));
    return out.filter((_, i) => i % every === 0);
  });
  const monthLabel = (m) =>
    new Date(Date.UTC(m.year, m.month, 1)).toLocaleDateString(undefined, {
      month: 'short',
      ...(m.month === 0 || m === months[0] ? { year: '2-digit' } : {}),
      timeZone: 'UTC',
    });
  const dateLabel = (day) =>
    new Date(day * 86400000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric', timeZone: 'UTC' });

  function move(e) {
    const r = e.currentTarget.getBoundingClientRect();
    const px = e.clientX - r.left;
    const day = x0 + ((px - M.l) / (width - M.l - M.r)) * (x1 - x0);
    let best = null;
    for (const s of series) {
      for (const p of s.points) {
        if (best == null || Math.abs(p.day - day) < Math.abs(best - day)) best = p.day;
      }
    }
    hover = best;
    pointer = { x: px, y: e.clientY - r.top };
  }

  const at = (s, day) => s.points.find((p) => p.day === day);
  const last = (s) => s.points.at(-1);
  // End labels only when they clear each other; otherwise the legend carries it.
  const endLabels = $derived.by(() => {
    if (series.length < 2) return false;
    const ys = series.map(last).filter(Boolean).map((p) => y(p.value)).sort((a, b) => a - b);
    return ys.every((v, i) => i === 0 || v - ys[i - 1] >= 14);
  });
</script>

<div class="wrap" bind:clientWidth={width}>
  {#if series.length > 1}
    <div class="legend">
      {#each series as s (s.key)}
        <span class="item"><span class="key" style:background={s.color}></span>{s.label}</span>
      {/each}
    </div>
  {/if}
  <svg {width} {height} role="img" aria-label={series.map((s) => s.label).join(', ')}>
    {#each scale.ticks as v (v)}
      <line class="grid" x1={M.l} x2={width - M.r} y1={y(v)} y2={y(v)} />
      <text class="tick tab" x={M.l - 8} y={y(v) + 4} text-anchor="end">{v}</text>
    {/each}
    <text class="tick" x={M.l - 8} y={10} text-anchor="end">{unit}</text>
    {#each months as m (m.day)}
      <text class="tick" x={x(m.day)} y={height - 6} text-anchor="middle">{monthLabel(m)}</text>
    {/each}
    {#each refs as r (r.label)}
      <line class="ref" x1={M.l} x2={width - M.r} y1={y(r.value)} y2={y(r.value)} />
      <text class="ref-label" x={width - M.r + 6} y={y(r.value) + 4}>{r.label}</text>
    {/each}
    {#each series as s (s.key)}
      {#if s.kind !== 'dots'}
        <path class="line" d={linePath(s.points.map((p) => [x(p.day), y(p.value)]))} style:stroke={s.color} />
      {/if}
      {#if s.kind !== 'line'}
        {#each s.points as p (p.day)}
          <circle class="dot" cx={x(p.day)} cy={y(p.value)} r="4" style:fill={s.color} style:opacity={p.faint ? 0.55 : 1} />
        {/each}
      {/if}
      {#if endLabels && last(s)}
        <text class="end-label" x={width - M.r + 6} y={y(last(s).value) + 4}>{s.label}</text>
      {/if}
    {/each}
    {#if hover != null}
      <line class="cross" x1={x(hover)} x2={x(hover)} y1={M.t} y2={height - M.b} />
    {/if}
    <rect
      class="hit"
      x={M.l}
      y={0}
      width={Math.max(0, width - M.l - M.r)}
      height={height - M.b}
      role="presentation"
      onpointermove={move}
      onpointerleave={() => (hover = null)}
    />
  </svg>
  {#if hover != null}
    <div class="tooltip" style:left="{Math.min(pointer.x + 14, width - 180)}px" style:top="{Math.max(0, pointer.y - 60)}px">
      <div class="row muted">{dateLabel(hover)}</div>
      {#each series as s (s.key)}
        {@const p = at(s, hover)}
        {#if p}
          <div class="row">
            <span class="key" style:background={s.color}></span><strong>{fixed(p.value, digits)}</strong>
            <span class="muted">{unit}{p.note ? ` · ${p.note}` : ''}</span>
          </div>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
    width: 100%;
  }
  svg {
    display: block;
    overflow: visible;
  }
  .legend {
    display: flex;
    gap: 1.25rem;
    font-size: 13px;
    color: var(--ink-2);
    margin-bottom: 0.25rem;
  }
  .item {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
  }
  .key {
    display: inline-block;
    width: 14px;
    height: 2px;
    border-radius: 1px;
  }
  .grid {
    stroke: var(--rule);
  }
  .tick {
    fill: var(--ink-3);
    font-size: 11px;
  }
  .ref {
    stroke: var(--ink-3);
    stroke-width: 1;
  }
  .ref-label,
  .end-label {
    fill: var(--ink-2);
    font-size: 12px;
  }
  .line {
    fill: none;
    stroke-width: 2;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .dot {
    stroke: var(--surface);
    stroke-width: 2;
  }
  .cross {
    stroke: var(--ink-3);
  }
  .hit {
    fill: transparent;
    cursor: crosshair;
  }
</style>
