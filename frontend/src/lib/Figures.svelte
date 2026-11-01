<script>
  // The selected stroke's body at the catch and the finish, over the
  // session's typical figure. Figures come in thigh lengths from the hip
  // (x toward the feet, y up), so they compare whatever the camera distance.
  import { BONES } from './pose.js';
  import { fixed } from './format.js';

  let { stroke, typicalCatch = [], typicalFinish = [], height = 200 } = $props();

  const H = $derived(height);
  const PAD = 0.25;

  const pose = $derived(stroke?.pose);
  const panels = $derived([
    { key: 'catch', title: 'Catch', mine: pose?.catch_figure ?? [], typical: typicalCatch, angles: pose?.catch },
    { key: 'finish', title: 'Finish', mine: pose?.finish_figure ?? [], typical: typicalFinish, angles: pose?.finish },
  ]);

  // One scale for all four figures, so catch and finish compare directly.
  const bounds = $derived.by(() => {
    const pts = panels.flatMap((p) => [...p.mine, ...p.typical]).filter(Boolean);
    if (!pts.length) return null;
    const xs = pts.map((p) => p[0]);
    const ys = pts.map((p) => p[1]);
    return {
      x0: Math.min(...xs) - PAD,
      x1: Math.max(...xs) + PAD,
      y0: Math.min(...ys) - PAD,
      y1: Math.max(...ys) + PAD,
    };
  });
  const scale = $derived(bounds ? H / (bounds.y1 - bounds.y0) : 1);
  const W = $derived(bounds ? (bounds.x1 - bounds.x0) * scale : 0);
  const px = (p) => [(p[0] - bounds.x0) * scale, (bounds.y1 - p[1]) * scale];

  const bones = (fig) =>
    BONES.filter(([a, b]) => fig[a] && fig[b]).map(([a, b]) => {
      const [x1, y1] = px(fig[a]);
      const [x2, y2] = px(fig[b]);
      return { x1, y1, x2, y2, key: `${a}-${b}` };
    });

  const deg = (v) => (v == null ? '–' : `${fixed(Math.abs(v))}°`);
  const lean = (v) => (v == null ? '–' : `${deg(v)} ${v >= 0 ? 'forward' : 'back'}`);
</script>

{#if bounds}
  <div class="legend">
    <span class="item"><span class="key" style:background="var(--accent)"></span>Stroke {stroke.index + 1}</span>
    <span class="item"><span class="key" style:background="var(--mean)"></span>Session typical</span>
  </div>
  <div class="panels">
    {#each panels as p (p.key)}
      <figure>
        <svg width={W} height={H} role="img" aria-label="{p.title}: body position of this stroke against the session's typical one">
          <line class="floor" x1="0" x2={W} y1={px([0, p.typical[6]?.[1] ?? 0])[1]} y2={px([0, p.typical[6]?.[1] ?? 0])[1]} />
          {#each bones(p.typical) as b (b.key)}
            <line class="typical" x1={b.x1} y1={b.y1} x2={b.x2} y2={b.y2} />
          {/each}
          {#each bones(p.mine) as b (b.key)}
            <line class="mine" x1={b.x1} y1={b.y1} x2={b.x2} y2={b.y2} />
          {/each}
          {#each p.mine.filter(Boolean) as pt, i (i)}
            {@const [x, y] = px(pt)}
            <circle class="joint" cx={x} cy={y} r="4" />
          {/each}
        </svg>
        <figcaption>
          <strong>{p.title}</strong>
          {#if p.key === 'catch'}
            <span>shin {deg(p.angles?.shin)} from vertical</span>
            <span>body {lean(p.angles?.trunk)}</span>
            <span>knee {deg(p.angles?.knee)}</span>
          {:else}
            <span>body {lean(p.angles?.trunk)}</span>
            <span>elbow {deg(p.angles?.elbow)}</span>
            <span>knee {deg(p.angles?.knee)}</span>
          {/if}
        </figcaption>
      </figure>
    {/each}
  </div>
{:else}
  <p class="muted">No camera view of this stroke.</p>
{/if}

<style>
  .legend {
    display: flex;
    gap: 1.25rem;
    font-size: 13px;
    color: var(--ink-2);
    margin-bottom: 0.5rem;
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
  .panels {
    display: flex;
    flex-wrap: wrap;
    gap: 2.5rem;
  }
  figure {
    margin: 0;
  }
  svg {
    display: block;
    overflow: visible;
  }
  .floor {
    stroke: var(--rule);
  }
  .typical,
  .mine {
    stroke-width: 2;
    stroke-linecap: round;
  }
  .typical {
    stroke: var(--mean);
  }
  .mine {
    stroke: var(--accent);
  }
  .joint {
    fill: var(--accent);
    stroke: var(--surface);
    stroke-width: 2;
  }
  figcaption {
    display: grid;
    gap: 0.1rem;
    margin-top: 0.6rem;
    font-size: 13px;
    color: var(--ink-2);
  }
  figcaption strong {
    color: var(--ink);
    font-weight: 600;
  }
</style>
