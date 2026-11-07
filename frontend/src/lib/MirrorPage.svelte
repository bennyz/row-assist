<script>
  // The row screen, in front of the erg: your body live over a target rower
  // doing a good stroke at the target rhythm, a rhythm bar, the numbers
  // against their targets, and the last stroke frozen at catch and finish.
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
  import { J, StrokeClock, measure, rhythmAt, targetPose } from './ghost.js';
  import MarkBar from './MarkBar.svelte';
  import NextRow from './NextRow.svelte';
  import PhoneSetup from './PhoneSetup.svelte';
  import RhythmBar from './RhythmBar.svelte';
  import RowBar from './RowBar.svelte';

  let { onsaved } = $props();

  const MODE_KEY = 'row-assist.mirror-mode';
  const CUE_SHOWN_MS = 6000;
  // Until your proportions are known: thigh lengths from the feet, x toward them.
  const DEFAULT_VIEW = { x0: -3.1, x1: 1.0, y0: -0.45, y1: 2.1 };
  const MARGIN = 0.35;
  // Frames your proportions are measured over, and how often.
  const MEASURE_FRAMES = 150;
  const MEASURE_EVERY = 30;
  // Longer than this since the last catch: not rowing, the target waits at the catch.
  const IDLE_S = 8;

  let mode = $state(readMode());
  let link = $state('connecting');
  let live = $state(false);
  let guides = $state(null);
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
  // Your proportions, for the target rower; and where you are in the stroke.
  let body = $state(null);
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
        guides = { catch: m.catch_trunk, finish: m.finish_trunk };
      } else if (m.type === 'figure') {
        joints = m.joints;
        recent.push(m.joints);
        if (recent.length > MEASURE_FRAMES) recent.shift();
        seen += 1;
        if (!body || seen % MEASURE_EVERY === 0) {
          body = measure(recent) ?? body;
          extent = reach(recent) ?? extent;
        }
        strokeClock.push(performance.now(), m.joints[J.WRIST]?.[0]);
        catchAt = strokeClock.catchAt;
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

  // The target stroke: the session's rate, else your own.
  const rhythm = $derived(targets?.rhythm ?? (last?.rate ? rhythmAt(last.rate) : null));
  const cycle = $derived(rhythm ? rhythm.drive_s + rhythm.recovery_s : 3);
  const driveShare = $derived(rhythm ? rhythm.drive_s / cycle : 1 / 3);
  const mid = (band, fallback) => (band ? (band[0] + band[1]) / 2 : fallback);
  const lean = $derived({ catch: mid(guides?.catch, 22.5), finish: mid(guides?.finish, -17.5) });
  // Seconds since your last catch, while rowing.
  const since = $derived(catchAt == null ? null : (now - catchAt) / 1000);
  const rowing = $derived(since != null && since < IDLE_S);
  // The target restarts at each of your catches: ahead of it means rushing.
  // Past its own stroke, it waits at the catch.
  const phase = $derived(rowing && since < cycle ? since / cycle : 0);
  const ghost = $derived(body ? targetPose(body, phase, driveShare, lean) : null);

  // Framed to the target's catch and finish, so nothing sits at an edge.
  const view = $derived.by(() => {
    if (!body) return DEFAULT_VIEW;
    const pts = [...targetPose(body, 0, driveShare, lean), ...targetPose(body, driveShare, driveShare, lean)].filter(
      Boolean,
    );
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
  const floorY = $derived(Math.min(body?.heel?.[1] ?? -0.15, body?.toe?.[1] ?? -0.15) - 0.06);

  const lines = (fig) =>
    fig
      ? BONES.filter(([a, b]) => fig[a] && fig[b]).map(([a, b]) => {
          const [x1, y1] = px(fig[a]);
          const [x2, y2] = px(fig[b]);
          return { x1, y1, x2, y2, key: `${a}-${b}` };
        })
      : [];
  const bones = $derived(lines(me));
  const ghostBones = $derived(lines(ghost));

  // Heart rate first: never "harder" while it's over the ceiling.
  const hrCeiling = $derived(targets?.hr_max != null ? Math.round(targets.hr_max) : null);
  const hrHigh = $derived(hrCeiling != null && last?.hr != null && last.hr > hrCeiling);
  const frozen = $derived(
    last?.catch
      ? { index: last.index, pose: { catch_figure: last.catch, finish_figure: last.finish, catch: last.catch_angles, finish: last.finish_angles } }
      : null,
  );
  const faultNames = $derived(last?.faults?.map((f) => FAULTS[f]?.label ?? f) ?? []);
</script>

<div class="mirror">
  <header>
    {#if row}<RowBar {row} minutes={targets?.minutes} onstop={stopRow} />{/if}
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
      <svg viewBox="0 0 {W} {H}" role="img" aria-label="Your body now, over a target rower doing a good stroke at the target rhythm">
        <line class="floor" x1="0" x2={W} y1={px([0, floorY])[1]} y2={px([0, floorY])[1]} />
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
      {#if ghost && me}
        <p class="legend"><span class="key you"></span>You <span class="key target"></span>Target: {rowing ? 'match its timing' : 'the catch'}</p>
      {/if}
      {#if rhythm && me}
        <RhythmBar {rhythm} sinceCatch={rowing ? since : null} last={last?.drive_s ? last : null} />
      {/if}
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
      {:else}
      <h2>Last stroke</h2>
      {#if frozen}
        <Figures stroke={frozen} typicalCatch={last.typical_catch} typicalFinish={last.typical_finish} height={110} />
      {:else}
        <p class="muted">Row a few strokes.</p>
      {/if}
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
  svg {
    display: block;
    width: 100%;
    height: auto;
    /* Everything on one screen, read from the erg: the figure gives way. */
    max-height: calc(100vh - 29rem);
    min-height: 14rem;
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
    /* Over the guides: they mean nothing without a body. */
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
    .numbers :global(.value) {
      font-size: 52px;
    }
  }
</style>
