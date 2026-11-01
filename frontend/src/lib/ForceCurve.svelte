<script>
  // One stroke's drive force against the session's mean curve, both resampled
  // to the same points so they line up from catch (0%) to finish (100%).
  import { fixed, linePath, niceTicks } from './format.js';

  let { stroke, mean = [] } = $props();

  const H = 220;
  const M = { l: 44, r: 16, t: 26, b: 26 };
  let width = $state(520);
  let hover = $state(null);
  let pointer = $state({ x: 0, y: 0 });

  const curve = $derived(stroke?.force_n ?? []);
  const n = $derived(Math.max(curve.length, mean.length));
  const scale = $derived(niceTicks(0, Math.max(1, ...curve, ...mean) * 1.05, 4));
  const x = (i) => M.l + (n > 1 ? i / (n - 1) : 0) * (width - M.l - M.r);
  const y = (v) => M.t + (H - M.t - M.b) * (1 - v / scale.hi);

  const strokePath = $derived(linePath(curve.map((v, i) => [x(i), y(v)])));
  const meanPath = $derived(linePath(mean.map((v, i) => [x(i), y(v)])));
  const area = $derived(
    curve.length ? `${strokePath}L${x(curve.length - 1).toFixed(1)},${y(0)}L${x(0).toFixed(1)},${y(0)}Z` : '',
  );
  const peakAt = $derived(curve.length ? curve.indexOf(Math.max(...curve)) : -1);

  function move(e) {
    const r = e.currentTarget.getBoundingClientRect();
    const px = e.clientX - r.left;
    hover = Math.round(((px - M.l) / (width - M.l - M.r)) * (n - 1));
    hover = Math.min(n - 1, Math.max(0, hover));
    pointer = { x: px, y: e.clientY - r.top };
  }
</script>

<div class="wrap" bind:clientWidth={width}>
  <div class="legend">
    <span class="item"><span class="key" style:background="var(--accent)"></span>Stroke {stroke ? stroke.index + 1 : ''}</span>
    <span class="item"><span class="key" style:background="var(--mean)"></span>Session mean</span>
  </div>
  {#if !curve.length && !mean.length}
    <p class="muted">No force data for this stroke.</p>
  {:else}
    <svg {width} height={H} role="img" aria-label="Force through the drive, this stroke against the session mean">
      {#each scale.ticks as v (v)}
        <line class="grid" x1={M.l} x2={width - M.r} y1={y(v)} y2={y(v)} />
        <text class="tick tab" x={M.l - 8} y={y(v) + 4} text-anchor="end">{v}</text>
      {/each}
      {#each [0, 25, 50, 75, 100] as p (p)}
        <text class="tick tab" x={x(((n - 1) * p) / 100)} y={H - 6} text-anchor="middle">{p}%</text>
      {/each}
      <text class="tick" x={M.l - 8} y={10} text-anchor="end">N</text>

      <path class="mean" d={meanPath} />
      <path class="wash" d={area} />
      <path class="stroke" d={strokePath} />
      {#if peakAt >= 0 && hover == null}
        <circle class="dot" cx={x(peakAt)} cy={y(curve[peakAt])} r="4" />
        <text class="label" x={x(peakAt)} y={y(curve[peakAt]) - 10} text-anchor="middle">
          {fixed(stroke?.peak_n ?? curve[peakAt])} N
        </text>
      {/if}
      {#if hover != null}
        <line class="cross" x1={x(hover)} x2={x(hover)} y1={M.t} y2={H - M.b} />
        {#if curve[hover] != null}<circle class="dot" cx={x(hover)} cy={y(curve[hover])} r="4" />{/if}
        {#if mean[hover] != null}<circle class="dot m" cx={x(hover)} cy={y(mean[hover])} r="4" />{/if}
      {/if}
      <rect
        class="hit"
        x={M.l}
        y={0}
        width={Math.max(0, width - M.l - M.r)}
        height={H - M.b}
        role="presentation"
        onpointermove={move}
        onpointerleave={() => (hover = null)}
      />
    </svg>
    {#if hover != null}
      <div class="tooltip" style:left="{Math.min(pointer.x + 14, width - 150)}px" style:top="{Math.max(0, pointer.y - 60)}px">
        <div class="row muted">{Math.round((hover / (n - 1)) * 100)}% of the drive</div>
        <div class="row"><span class="key" style:background="var(--accent)"></span><strong>{fixed(curve[hover])}</strong><span class="muted">N stroke</span></div>
        <div class="row"><span class="key" style:background="var(--mean)"></span><strong>{fixed(mean[hover])}</strong><span class="muted">N mean</span></div>
      </div>
    {/if}
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
  /* Raw peak (resampling can shave it); a surface halo keeps it readable over the mean line. */
  .label {
    fill: var(--ink);
    font-size: 12px;
    font-family: var(--font-num);
    paint-order: stroke;
    stroke: var(--surface);
    stroke-width: 4px;
    stroke-linejoin: round;
  }
  .mean,
  .stroke {
    fill: none;
    stroke-width: 2;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .mean {
    stroke: var(--mean);
  }
  .stroke {
    stroke: var(--accent);
  }
  .wash {
    fill: var(--accent-wash);
  }
  .cross {
    stroke: var(--ink-3);
  }
  .dot {
    fill: var(--accent);
    stroke: var(--surface);
    stroke-width: 2;
  }
  .dot.m {
    fill: var(--mean);
  }
  .hit {
    fill: transparent;
    cursor: crosshair;
  }
</style>
