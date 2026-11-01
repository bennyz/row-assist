<script>
  // The front screen: your body live, with guides, and the last stroke frozen
  // at catch and finish. Read from the erg, so everything is big. Figures and
  // guides come from the server (/ws/live); this page only draws.
  import { onDestroy, onMount } from 'svelte';
  import { BONES } from './pose.js';
  import { FAULTS, fixed } from './format.js';
  import Figures from './Figures.svelte';

  const MODE_KEY = 'row-assist.mirror-mode';
  const CUE_SHOWN_MS = 6000;
  // Live view bounds in thigh lengths, ankle at the origin (x toward the feet).
  const VIEW = { x0: -3.1, x1: 1.0, y0: -0.45, y1: 2.1 };

  let mode = $state(readMode());
  let link = $state('connecting');
  let live = $state(false);
  let guides = $state(null);
  let joints = $state(null);
  let last = $state(null);
  let focus = $state(null);
  let cue = $state(null);
  let cueTimer;
  let ws;
  let alive = true;

  function readMode() {
    try {
      return localStorage.getItem(MODE_KEY) ?? 'mirror';
    } catch {
      return 'mirror';
    }
  }

  function setMode(m) {
    mode = m;
    try {
      localStorage.setItem(MODE_KEY, m);
    } catch {
      // Not remembered in private mode.
    }
  }

  function connect() {
    if (!alive) return;
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    ws = new WebSocket(`${proto}://${location.host}/ws/live`);
    ws.onmessage = (e) => {
      const m = JSON.parse(e.data);
      if (m.type === 'hello') {
        link = 'open';
        live = m.live;
        guides = { catch: m.catch_trunk, finish: m.finish_trunk };
      } else if (m.type === 'figure') {
        joints = m.joints;
      } else if (m.type === 'stroke') {
        last = m;
      } else if (m.type === 'focus') {
        focus = m;
      } else if (m.type === 'cue') {
        cue = m.text;
        clearTimeout(cueTimer);
        cueTimer = setTimeout(() => (cue = null), CUE_SHOWN_MS);
      }
    };
    ws.onclose = () => {
      link = 'closed';
      if (alive) setTimeout(connect, 2000);
    };
  }

  onMount(connect);
  onDestroy(() => {
    alive = false;
    ws?.close();
    clearTimeout(cueTimer);
  });

  // Live figure, re-anchored at the ankle so the body slides like on the erg.
  const anchored = $derived.by(() => {
    const ankle = joints?.[6];
    if (!ankle) return null;
    return joints.map((p) => (p ? [p[0] - ankle[0], p[1] - ankle[1]] : null));
  });

  const W = 820;
  const S = W / (VIEW.x1 - VIEW.x0);
  const H = (VIEW.y1 - VIEW.y0) * S;
  const px = (p) => [(p[0] - VIEW.x0) * S, (VIEW.y1 - p[1]) * S];

  const bones = $derived(
    anchored
      ? BONES.filter(([a, b]) => anchored[a] && anchored[b]).map(([a, b]) => {
          const [x1, y1] = px(anchored[a]);
          const [x2, y2] = px(anchored[b]);
          return { x1, y1, x2, y2, key: `${a}-${b}` };
        })
      : [],
  );

  /** A wedge from the hip between two body angles (degrees, + toward the feet). */
  function wedge(hip, [from, to], r) {
    const at = (deg) => {
      const a = (deg * Math.PI) / 180;
      return px([hip[0] + r * Math.sin(a), hip[1] + r * Math.cos(a)]);
    };
    const [hx, hy] = px(hip);
    const [ax, ay] = at(from);
    const [bx, by] = at(to);
    return `M${hx},${hy}L${ax},${ay}A${r * S},${r * S} 0 0 ${from < to ? 1 : 0} ${bx},${by}Z`;
  }

  const torso = $derived(
    anchored?.[1] && anchored?.[4] ? Math.hypot(anchored[1][0] - anchored[4][0], anchored[1][1] - anchored[4][1]) : 1.3,
  );
  const frozen = $derived(
    last?.catch
      ? { index: last.index, pose: { catch_figure: last.catch, finish_figure: last.finish, catch: last.catch_angles, finish: last.finish_angles } }
      : null,
  );
  const faultNames = $derived(last?.faults?.map((f) => FAULTS[f]?.label ?? f) ?? []);
</script>

<div class="mirror">
  <header>
    <dl class="numbers">
      <div><dd class="num">{fixed(last?.rate, 1)}</dd><dt>spm</dt></div>
      <div><dd class="num">{fixed(last?.watts)}</dd><dt>watts</dt></div>
      <div>
        <dd class="num">{fixed(last?.hr)}{#if last?.zone != null}<span class="zone">Z{last.zone}</span>{/if}</dd>
        <dt>heart rate</dt>
      </div>
    </dl>
    <div class="mode" role="group" aria-label="What to show">
      <button aria-pressed={mode === 'mirror'} onclick={() => setMode('mirror')}>Mirror</button>
      <button aria-pressed={mode === 'numbers'} onclick={() => setMode('numbers')}>Numbers only</button>
    </div>
  </header>

  {#if link !== 'open'}
    <p class="state">Connecting to Row Assist…</p>
  {:else if !live}
    <p class="state">No session running. Start one on the Mac with <code>rowctl record --camera</code>.</p>
  {:else if mode === 'mirror'}
    <div class="stage">
      <svg viewBox="0 0 {W} {H}" role="img" aria-label="Your body now, with target guides">
        {#if anchored?.[4] && guides}
          {@const hip = anchored[4]}
          <path class="guide" d={wedge(hip, guides.catch, torso)} />
          <path class="guide" d={wedge(hip, guides.finish, torso)} />
        {/if}
        <!-- Shin vertical: the knee over the ankle at the catch. -->
        <line class="guide-line" x1={px([0, 0])[0]} y1={px([0, 0])[1]} x2={px([0, 1.15])[0]} y2={px([0, 1.15])[1]} />
        <line class="floor" x1="0" x2={W} y1={px([0, -0.25])[1]} y2={px([0, -0.25])[1]} />
        {#each bones as b (b.key)}
          <line class="bone" x1={b.x1} y1={b.y1} x2={b.x2} y2={b.y2} />
        {/each}
        {#each (anchored ?? []).filter(Boolean) as p, i (i)}
          {@const [x, y] = px(p)}
          <circle class="joint" cx={x} cy={y} r="9" />
        {/each}
      </svg>
      {#if !anchored}
        <p class="overlay">Waiting for the camera: open /camera on the Mac.</p>
      {/if}
    </div>

    <aside>
      <h2>Last stroke</h2>
      {#if frozen}
        <Figures stroke={frozen} typicalCatch={last.typical_catch} typicalFinish={last.typical_finish} height={170} />
      {:else}
        <p class="muted">Row a few strokes.</p>
      {/if}
    </aside>
  {/if}

  <footer>
    {#if cue}
      <p class="cue">{cue}</p>
    {:else if faultNames.length}
      <p class="faults">{faultNames.join(' · ')}</p>
    {/if}
    {#if focus?.fault}
      <p class="focus">Focus: {FAULTS[focus.fault]?.label} — <span>{focus.cue}</span></p>
    {/if}
  </footer>
</div>

<style>
  .mirror {
    min-height: 100vh;
    display: grid;
    grid-template-columns: minmax(0, 2fr) minmax(16rem, 1fr);
    grid-template-rows: auto 1fr auto;
    gap: 1.5rem 2.5rem;
    padding: 1.5rem 2.5rem;
    background: var(--surface);
  }
  header {
    grid-column: 1 / -1;
    display: flex;
    justify-content: space-between;
    align-items: start;
  }
  .numbers {
    display: flex;
    gap: 3.5rem;
    margin: 0;
  }
  .numbers dd {
    margin: 0;
    font-size: 88px;
    font-weight: 600;
    line-height: 1;
  }
  .numbers dt {
    margin-top: 0.35rem;
    color: var(--ink-3);
    font-size: 16px;
  }
  .zone {
    font-size: 36px;
    color: var(--ink-2);
    margin-left: 0.6rem;
  }
  .mode {
    display: flex;
    gap: 0.5rem;
  }
  .mode button[aria-pressed='true'] {
    color: var(--ink);
    border-color: var(--accent);
  }
  .state {
    grid-column: 1 / -1;
    font-size: 22px;
    color: var(--ink-2);
  }
  .stage {
    position: relative;
    align-self: center;
  }
  svg {
    display: block;
    width: 100%;
    height: auto;
  }
  .guide {
    fill: var(--accent-wash);
    stroke: var(--rule);
  }
  .guide-line {
    stroke: var(--ink-3);
    stroke-width: 2;
  }
  .floor {
    stroke: var(--rule);
    stroke-width: 2;
  }
  .bone {
    stroke: var(--accent);
    stroke-width: 10;
    stroke-linecap: round;
  }
  .joint {
    fill: var(--accent);
    stroke: var(--surface);
    stroke-width: 4;
  }
  .overlay {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    margin: 0;
    color: var(--ink-2);
    font-size: 20px;
  }
  aside h2 {
    font-size: 15px;
    font-weight: 500;
    color: var(--ink-3);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 0 0 1rem;
  }
  footer {
    grid-column: 1 / -1;
    min-height: 4.5rem;
  }
  .cue {
    margin: 0;
    font: 600 44px var(--font-num);
    color: var(--accent);
  }
  .faults {
    margin: 0;
    font-size: 24px;
    color: var(--ink-2);
  }
  .focus {
    margin: 0.4rem 0 0;
    font-size: 18px;
    color: var(--ink-3);
  }
  .focus span {
    color: var(--ink-2);
  }
  code {
    font-family: var(--font-num);
    color: var(--ink);
  }
  @media (max-width: 900px) {
    .mirror {
      grid-template-columns: minmax(0, 1fr);
      padding: 1rem;
    }
    .numbers {
      gap: 1.5rem;
    }
    .numbers dd {
      font-size: 52px;
    }
  }
</style>
