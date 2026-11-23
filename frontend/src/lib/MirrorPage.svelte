<script>
  // The row screen, in front of the erg: your body live beside a pace ghost,
  // the good stroke on your body rowed at the rhythm to row at from each of
  // your catches (row with it: where you differ is the order your legs, body
  // and arms move in, or your timing), the numbers against their targets, and
  // in rests and between rows the last stroke: the order it was rowed in, and
  // its catch and finish frozen. The ghost's rate is the coach's, which
  // slows when heart rate stays over a ceiling.
  // With `rowctl serve` it also shows the row's state and devices, the next
  // row's plan before the first stroke, and the phone's code while no camera
  // is connected; when a row is saved it hands over to the review
  // (`onsaved`). Read from the erg, so everything is big. Everything comes
  // from the server (/ws/live); this page only draws.
  import { onDestroy, onMount } from 'svelte';
  import { BONES } from './pose.js';
  import CameraStill from './CameraStill.svelte';
  import { FAULTS } from './format.js';
  import { stopRow } from './api.js';
  import Figures from './Figures.svelte';
  import Gauge from './Gauge.svelte';
  import MarkBar from './MarkBar.svelte';
  import NextRow from './NextRow.svelte';
  import PhoneSetup from './PhoneSetup.svelte';
  import RowBar from './RowBar.svelte';
  import Sequence from './Sequence.svelte';
  import { StrokeClock, WRIST, rhythmAt } from './strokeclock.js';
  import { keepScreenOn } from './wake.js';

  let { onsaved } = $props();

  const MODE_KEY = 'row-assist.mirror-mode';
  const CUE_SHOWN_MS = 6000;
  // Until your proportions are known: thigh lengths from the feet, x toward them.
  const DEFAULT_VIEW = { x0: -3.1, x1: 1.0, y0: -0.45, y1: 2.1 };
  const MARGIN = 0.35;
  // Frames your range of movement is taken over, and how often.
  const EXTENT_FRAMES = 150;
  const EXTENT_EVERY = 30;
  // Longer than this since the last catch: not rowing, no place in the rhythm.
  const IDLE_S = 8;
  // Recovery this much shorter than the ghost's: "slow the slide".
  const RUSHED_S = 0.25;

  let mode = $state(readMode());
  let link = $state('connecting');
  let live = $state(false);
  let joints = $state(null);
  let last = $state(null);
  let focus = $state(null);
  let cue = $state(null);
  let cueTimer;
  // The camera's setup check; gone when the camera page stops sending it.
  let setup = $state(null);
  let setupTimer;
  // The row `rowctl serve` runs (null with `rowctl record`).
  let row = $state(null);
  // How the last row ended when this page opened: only later ones hand over.
  let seenEnd;
  let note = $state(null);
  // What the row aims for: bands, and the rhythm at the target rate.
  let targets = $state(null);
  // The good stroke on your body: its catch and finish, and its order.
  let target = $state(null);
  // How far you actually move (recent frames), so the view never cuts you off.
  let extent = $state(null);
  let now = $state(performance.now());
  let catchAt = $state(null);
  const recent = [];
  let seen = 0;
  const strokeClock = new StrokeClock();
  let ws;
  let alive = true;
  let raf;

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
      } else if (m.type === 'figure') {
        joints = m.joints;
        recent.push(m.joints);
        if (recent.length > EXTENT_FRAMES) recent.shift();
        seen += 1;
        if (!extent || seen % EXTENT_EVERY === 0) extent = reach(recent) ?? extent;
        strokeClock.push(performance.now(), m.joints[WRIST]?.[0]);
        catchAt = strokeClock.catchAt;
      } else if (m.type === 'target') {
        target = m;
      } else if (m.type === 'targets') {
        targets = m;
      } else if (m.type === 'stroke') {
        last = m;
      } else if (m.type === 'focus') {
        focus = m;
      } else if (m.type === 'framing') {
        setup = m;
        clearTimeout(setupTimer);
        setupTimer = setTimeout(() => (setup = null), 2000);
      } else if (m.type === 'row') {
        const end = m.last?.at_us ?? null;
        if (seenEnd === undefined) {
          seenEnd = end;
        } else if (end !== seenEnd) {
          seenEnd = end;
          if (m.last.session != null) onsaved?.(m.last.session);
          else note = `${m.last.strokes} strokes: too short to keep.`;
        }
        if (m.phase === 'rowing') note = null;
        row = m;
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

  /** The box your recent frames fill, ignoring the odd stray point. */
  function reach(frames) {
    const xs = [];
    const ys = [];
    for (const f of frames) for (const p of f ?? []) if (p) (xs.push(p[0]), ys.push(p[1]));
    if (xs.length < 100) return null;
    const q = (v, k) => v.sort((a, b) => a - b)[Math.floor(k * (v.length - 1))];
    return { x0: q(xs, 0.01), x1: q(xs, 0.99), y0: q(ys, 0.01), y1: q(ys, 0.99) };
  }

  function animate() {
    now = performance.now();
    if (alive) raf = requestAnimationFrame(animate);
  }

  // The laptop in front of the erg stays on while the row screen is up.
  onMount(() => keepScreenOn());

  onMount(() => {
    connect();
    raf = requestAnimationFrame(animate);
  });
  onDestroy(() => {
    alive = false;
    ws?.close();
    cancelAnimationFrame(raf);
    clearTimeout(cueTimer);
    clearTimeout(setupTimer);
  });

  // The live figure comes from where your feet rest, so it holds still while
  // the seat slides.
  const me = $derived(joints?.some(Boolean) ? joints : null);

  // The rhythm to row at: the session's rate, else your own.
  const rhythm = $derived(targets?.rhythm ?? (last?.rate ? rhythmAt(last.rate) : null));
  // Seconds since your last catch, while rowing: your place in the rhythm.
  const since = $derived(catchAt == null ? null : (now - catchAt) / 1000);
  const rowing = $derived(since != null && since < IDLE_S);
  // Where the workout is (none before Go); the ghost sits out the rests.
  const position = $derived(targets?.position ?? null);
  // The pace ghost: the good stroke on your body, from your last catch at the
  // rhythm to row at (on the live figure's own clock, so it shares its
  // delay). Rowing with it, you overlap; rushing the slide, it is still
  // coming forward when you catch; late, it waits at the catch.
  const paceGhost = $derived.by(() => {
    const path = target?.path;
    if (!path || !rhythm || !me || position?.stage === 'rest') return null;
    const atCatch = path.recovery[path.recovery.length - 1];
    if (!rowing) return atCatch;
    if (since < rhythm.drive_s) return along(path.drive, driveEase(since / rhythm.drive_s));
    const r = since - rhythm.drive_s;
    return r < rhythm.recovery_s ? along(path.recovery, recoveryEase(r / rhythm.recovery_s)) : atCatch;
  });
  // The handle through the drive: speeding up, then easing into the finish.
  const driveEase = (x) => x * x * (3 - 2 * x);
  // Through the recovery: hands away and body over quickly, then a slow slide.
  const recoveryEase = (x) => 1 - (1 - x) ** 1.6;
  /** The figure `u` of the way along a path of figures, between its steps. */
  function along(frames, u) {
    const k = Math.min(frames.length - 1, Math.max(0, u) * (frames.length - 1));
    const i = Math.floor(k);
    const a = frames[i];
    const b = frames[Math.min(i + 1, frames.length - 1)];
    const f = k - i;
    return a.map((p, j) => (p && b[j] ? [p[0] + (b[j][0] - p[0]) * f, p[1] + (b[j][1] - p[1]) * f] : p));
  }
  // One word on the timing, from your last stroke: rushing the slide is the fault.
  const timing = $derived.by(() => {
    if (!rhythm || !rowing || !last?.recovery_s) return null;
    const short = rhythm.recovery_s - last.recovery_s;
    return short > RUSHED_S
      ? { tone: 'warn', text: `Slow the slide: ${short.toFixed(1)} s more coming forward` }
      : { tone: 'ok', text: 'On rhythm' };
  });
  // The last stroke's order and frozen figures: for rests and between rows.
  const showLast = $derived(!rowing || position?.stage === 'rest');

  // Framed to the target's catch and finish, so nothing sits at an edge.
  const view = $derived.by(() => {
    if (!target) return DEFAULT_VIEW;
    const pts = [...target.catch, ...target.finish].filter(Boolean);
    const xs = pts.map((p) => p[0]).concat(extent ? [extent.x0, extent.x1] : []);
    const ys = pts.map((p) => p[1]).concat(extent ? [extent.y0, extent.y1] : []);
    return {
      x0: Math.min(...xs) - MARGIN,
      x1: Math.max(...xs) + MARGIN,
      y0: Math.min(...ys) - MARGIN,
      y1: Math.max(...ys) + MARGIN,
    };
  });
  const W = 820;
  const S = $derived(W / (view.x1 - view.x0));
  const H = $derived((view.y1 - view.y0) * S);
  const px = (p) => [(p[0] - view.x0) * S, (view.y1 - p[1]) * S];
  const floorY = $derived(Math.min(target?.catch[7]?.[1] ?? -0.15, target?.catch[8]?.[1] ?? -0.15) - 0.06);
  // How far the handle should travel: the good stroke's hands at the catch and at the finish.
  const rail = $derived(
    target?.catch[WRIST] && target?.finish[WRIST] ? { catch: px(target.catch[WRIST]), finish: px(target.finish[WRIST]) } : null,
  );

  const lines = (fig) =>
    fig
      ? BONES.filter(([a, b]) => fig[a] && fig[b]).map(([a, b]) => {
          const [x1, y1] = px(fig[a]);
          const [x2, y2] = px(fig[b]);
          return { x1, y1, x2, y2, key: `${a}-${b}` };
        })
      : [];
  const bones = $derived(lines(me));
  const ghostBones = $derived(lines(paceGhost));

  // Heart rate first: never "harder" while it's over the ceiling.
  const hrCeiling = $derived(targets?.hr_max != null ? Math.round(targets.hr_max) : null);
  const hrHigh = $derived(hrCeiling != null && last?.hr != null && last.hr > hrCeiling);
  const frozen = $derived(
    last?.catch
      ? { index: last.index, pose: { catch_figure: last.catch, finish_figure: last.finish, catch: last.catch_angles, finish: last.finish_angles } }
      : null,
  );
  // What the last stroke showed, each with what to do about it.
  const faults = $derived(last?.faults ?? []);
</script>

<div class="mirror" class:wide={mode === 'mirror' && row?.phase === 'rowing' && !showLast && !(setup && !setup.ok) && !note}>
  <header>
    {#if row}<RowBar {row} position={targets?.position} onstop={stopRow} />{/if}
    <div class="top">
    <div class="numbers">
      <Gauge label="spm" value={last?.rate ?? null} digits={1} band={targets?.rate} hints={{ low: 'quicker', high: 'slower' }} />
      <Gauge label="watts" value={last?.watts ?? null} band={targets?.watts} hints={{ low: hrHigh ? null : 'harder', high: 'easier' }} />
      <Gauge
        label="heart rate"
        value={last?.hr ?? null}
        band={hrCeiling != null ? [null, hrCeiling] : null}
        extra={last?.zone != null ? `Z${last.zone}` : null}
        hints={{ high: 'ease off' }}
      />
    </div>
    <div class="mode" role="group" aria-label="What to show">
      <button aria-pressed={mode === 'mirror'} onclick={() => setMode('mirror')}>Mirror</button>
      <button aria-pressed={mode === 'numbers'} onclick={() => setMode('numbers')}>Numbers only</button>
      {#if row}<a class="review" href="/sessions">Sessions</a>{/if}
    </div>
    </div>
  </header>

  {#if link !== 'open'}
    <p class="state">Connecting to Row Assist…</p>
  {:else if !live}
    <p class="state">Row Assist isn’t running. Start it with <code>rowctl serve</code>.</p>
  {:else if mode === 'mirror'}
    <div class="stage">
      <svg viewBox="0 0 {W} {H}" role="img" aria-label="Your body now, beside the good stroke rowed at the rhythm to row at">
        <line class="floor" x1="0" x2={W} y1={px([0, floorY])[1]} y2={px([0, floorY])[1]} />
        {#if rail && me}
          <line class="rail" x1={rail.catch[0]} y1={rail.catch[1]} x2={rail.finish[0]} y2={rail.finish[1]} />
          {#each [['catch', rail.catch], ['finish', rail.finish]] as [name, [x, y]] (name)}
            <line class="rail-end" x1={x} x2={x} y1={y - 16} y2={y + 16} />
            <text class="rail-name" {x} y={y - 24} text-anchor="middle">{name}</text>
          {/each}
        {/if}
        {#each ghostBones as b (b.key)}
          <line class="ghost" x1={b.x1} y1={b.y1} x2={b.x2} y2={b.y2} />
        {/each}
        {#each bones as b (b.key)}
          <line class="bone" x1={b.x1} y1={b.y1} x2={b.x2} y2={b.y2} />
        {/each}
        {#each (me ?? []).filter(Boolean) as p, i (i)}
          {@const [x, y] = px(p)}
          <circle class="joint" cx={x} cy={y} r="9" />
        {/each}
      </svg>
      {#if paceGhost}
        <p class="legend">
          <span class="key you"></span>You <span class="key target"></span>Row with the grey figure: rate {Math.round(rhythm.rate)}
        </p>
      {/if}
      {#if timing}<p class="timing {timing.tone}">{timing.text}</p>{/if}
      {#if !me}
        {#if row && row.devices.camera.connected !== true}
          <div class="overlay"><PhoneSetup /></div>
        {:else}
          <p class="overlay">
            {setup ? 'The camera is on but can’t see you yet.' : 'Waiting for the camera: open /camera on this computer, or scan the code rowctl printed with the phone.'}
          </p>
        {/if}
      {/if}
    </div>

    <aside>
      {#if setup && !setup.ok}
        <section class="setup" aria-live="polite">
          <h2>Camera setup</h2>
          <ul>
            {#each setup.checks.filter((c) => !c.ok) as c (c.key)}
              <li><span aria-hidden="true">✕</span> {c.detail}</li>
            {/each}
          </ul>
        </section>
      {/if}
      {#if row?.devices.camera.connected === true && row.phase !== 'rowing'}
        <CameraStill />
      {/if}
      {#if note}<p class="note">{note}</p>{/if}
      {#if row?.phase === 'ready' && row.plan}
        <NextRow plan={row.plan} watchAt={row.watch_at_us} />
      {:else if showLast}
      <h2>Last stroke</h2>
      {#if frozen}
        {#if target}
          <div class="order"><Sequence sequence={last.sequence} target={target.sequence} faults={faults.map((f) => f.fault)} /></div>
        {/if}
        <div class="frozen">
          <Figures stroke={frozen} typicalCatch={last.typical_catch} typicalFinish={last.typical_finish} height={110} />
        </div>
      {:else}
        <p class="muted">Row a few strokes.</p>
      {/if}
      {/if}
    </aside>
  {/if}

  <footer>
    {#if cue}
      <p class="cue">{cue}</p>
    {/if}
    {#if focus?.fault}
      <p class="focus">Focus: {FAULTS[focus.fault]?.label} — <span>{focus.cue}</span></p>
    {/if}
    {#if row}<div class="marks"><MarkBar mark={row.mark} /></div>{/if}
  </footer>
</div>

<style>
  .mirror {
    min-height: 100vh;
    display: grid;
    grid-template-columns: minmax(0, 2fr) minmax(16rem, 1fr);
    grid-template-rows: auto 1fr auto;
    gap: 1rem 2.5rem;
    padding: 1rem 2.5rem;
    background: var(--surface);
  }
  header {
    grid-column: 1 / -1;
    display: grid;
    gap: 1.25rem;
  }
  .top {
    display: flex;
    justify-content: space-between;
    align-items: start;
  }
  .note {
    margin: 0 0 1.5rem;
    font-size: 18px;
    color: var(--ink-2);
  }
  .numbers {
    display: flex;
    gap: 3.5rem;
  }
  .mode {
    display: flex;
    gap: 0.5rem;
  }
  .review {
    align-self: center;
    margin-left: 0.75rem;
    color: var(--ink-2);
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
    align-self: start;
  }
  /* Rowing, with nothing beside it: the figure and its ghost take the width. */
  .mirror.wide {
    grid-template-columns: minmax(0, 1fr);
  }
  .mirror.wide aside {
    display: none;
  }
  .timing {
    margin: 0.75rem 0 0;
    font-size: 22px;
  }
  .timing.ok {
    color: var(--ok);
  }
  .timing.warn {
    color: var(--warn);
  }
  svg {
    display: block;
    width: 100%;
    height: auto;
    /* Everything on one screen, read from the erg: the figure gives way to
       the numbers above it and the coach's words below. */
    max-height: calc(100vh - 25rem);
    min-height: 16rem;
  }
  .marks {
    margin-top: 0.75rem;
  }
  .setup {
    margin-bottom: 1.5rem;
  }
  .setup ul {
    list-style: none;
    padding: 0;
    margin: 0;
    font-size: 20px;
    color: var(--ink);
  }
  .setup li {
    margin-bottom: 0.4rem;
  }
  .setup span {
    color: var(--bad);
    margin-right: 0.4rem;
  }
  .ghost {
    stroke: var(--mean);
    stroke-width: 22;
    stroke-linecap: round;
    opacity: 0.45;
  }
  .legend {
    position: absolute;
    top: 0;
    left: 0;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0;
    font-size: 16px;
    color: var(--ink-3);
  }
  .key {
    display: inline-block;
    width: 1.4rem;
    height: 6px;
    border-radius: 3px;
  }
  .key.you {
    background: var(--accent);
  }
  .key.target {
    margin-left: 1rem;
    background: var(--mean);
    opacity: 0.7;
  }
  .floor {
    stroke: var(--rule);
    stroke-width: 2;
  }
  .rail {
    stroke: var(--mean);
    stroke-width: 2;
    stroke-dasharray: 4 10;
    opacity: 0.6;
  }
  .rail-end {
    stroke: var(--mean);
    stroke-width: 3;
    stroke-linecap: round;
  }
  .rail-name {
    fill: var(--ink-3);
    font-size: 17px;
  }
  .bone {
    stroke: var(--accent);
    stroke-width: 10;
    stroke-linecap: round;
  }
  .order {
    margin-bottom: 1.5rem;
  }
  /* A short screen keeps the stroke's order and drops its frozen catch and
     finish: the page mustn't scroll mid-row. */
  @media (max-height: 1000px) {
    .frozen {
      display: none;
    }
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
    /* Over the stage: its marks mean nothing without a body. */
    background: var(--surface);
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
    .numbers :global(.value) {
      font-size: 52px;
    }
  }
</style>
