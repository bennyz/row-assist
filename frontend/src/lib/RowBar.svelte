<script>
  // The row's state, the workout's Start and where it is, and the devices,
  // across the top of the row screen. Read from the erg: one line, big.
  // Everything comes from the server's `row` message and the coach's
  // position; the countdowns are only display.
  import { startWorkout } from './api.js';
  import { KINDS, clock } from './format.js';

  // `position`: where the workout is once started ({stage, n, of, left_s}).
  let { row, position = null, onstop } = $props();

  // A rest shorter than this isn't worth a countdown.
  const COUNTDOWN_AFTER_S = 15;

  const ending = $derived(
    row.phase === 'rowing' && row.idle_s != null && row.idle_s >= COUNTDOWN_AFTER_S
      ? Math.max(0, row.ends_after_s - row.idle_s)
      : null,
  );
  const w = $derived(row.workout);
  const name = $derived(KINDS[row.plan?.kind] ?? 'workout');
  const canStart = $derived(w.state === 'waiting' && row.phase !== 'saving');
  const stageLabel = $derived(
    position == null
      ? null
      : position.stage === 'rest'
        ? 'Rest'
        : position.of > 1
          ? `Work ${position.n} of ${position.of}`
          : name,
  );

  let failed = $state(false);
  async function start() {
    failed = false;
    const res = await startWorkout().catch(() => null);
    failed = !res?.ok;
  }
  // Space or Enter starts it, unless a control has the key.
  function key(e) {
    if (!canStart || (e.key !== ' ' && e.key !== 'Enter')) return;
    if (e.target.closest?.('button, input, textarea, select, a')) return;
    e.preventDefault();
    start();
  }

  const devices = $derived([
    { name: 'PM5', link: row.devices.pm5 },
    { name: row.hr != null && row.devices.h10.connected ? `Heart ${row.hr}` : 'Heart', link: row.devices.h10 },
    { name: 'Camera', link: row.devices.camera },
  ]);
  const mark = (c) => (c === true ? '✓' : c === false ? '✕' : '…');
  const tone = (c) => (c === true ? 'ok' : c === false ? 'bad' : 'muted');
</script>

<svelte:window onkeydown={key} />

<div class="bar">
  <p class="what" aria-live="polite">
    {#if row.phase === 'saving'}
      <strong>Saving…</strong>
    {:else}
      {#if row.phase === 'rowing'}
        <strong class="rec"><span aria-hidden="true">●</span> Recording</strong>
        <span class="num">{clock(row.elapsed_s)}</span> · {row.strokes} strokes ·
      {/if}
      {#if w.state === 'waiting'}
        <strong>{row.phase === 'ready' ? 'Ready.' : 'Warming up.'}</strong> Press Start for the {name.toLowerCase()}.
      {:else if w.state === 'countdown'}
        <strong class="go">Go in <span class="num">{Math.ceil(w.left_s)}</span></strong>
      {:else if w.state === 'running' && stageLabel}
        <span class="stage {position.stage}">{stageLabel}</span>{#if position.left_s != null}{' '}<span class="num">{clock(position.left_s)}</span> left{/if}
      {:else if w.state === 'done'}
        <strong>Done.</strong> Cool down; it saves when you stop.
      {/if}
      {#if ending != null}<span class="ending">· saves in {clock(ending)} unless you row</span>{/if}
    {/if}
  </p>
  <ul class="devices" aria-label="Devices">
    {#each devices as d (d.name)}
      <li class={tone(d.link.connected)} title={d.link.detail ?? ''}>
        <span aria-hidden="true">{mark(d.link.connected)}</span>
        {d.name}<span class="sr">{d.link.connected === true ? ' connected' : d.link.connected === false ? ' not connected' : ' waiting'}</span>
      </li>
    {/each}
  </ul>
  {#if canStart}
    <button class="start" onclick={start}>Start</button>
  {/if}
  {#if row.phase === 'rowing'}
    <button class="stop" onclick={onstop}>Stop and save</button>
  {/if}
  {#if failed}<p class="failed" role="alert">Not started: is Row Assist running?</p>{/if}
</div>

<style>
  .start {
    padding: 0.55rem 1.6rem;
    font-size: 20px;
    font-weight: 600;
    color: var(--bg);
    background: var(--accent);
    border-color: var(--accent);
  }
  .start:hover {
    filter: brightness(1.08);
  }
  .what .go {
    color: var(--accent);
  }
  .failed {
    margin: 0;
    color: var(--bad);
  }
  .stage {
    font-weight: 600;
  }
  .stage.work {
    color: var(--accent);
  }
  .stage.rest,
  .stage.done {
    color: var(--ink-2);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 1.5rem 2rem;
    flex-wrap: wrap;
    padding-bottom: 1rem;
    border-bottom: 1px solid var(--rule);
  }
  .what {
    flex: 1 1 24rem;
    margin: 0;
    font-size: 22px;
    color: var(--ink-2);
  }
  .what strong {
    color: var(--ink);
    font-weight: 600;
  }
  .rec span {
    color: var(--bad);
  }
  .num {
    font-family: var(--font-num);
    color: var(--ink);
  }
  .ending {
    color: var(--warn);
  }
  .devices {
    display: flex;
    gap: 1.25rem;
    list-style: none;
    margin: 0;
    padding: 0;
    font-size: 18px;
  }
  .devices li {
    color: var(--ink-2);
  }
  .devices span[aria-hidden] {
    margin-right: 0.3rem;
  }
  .ok span[aria-hidden] {
    color: var(--ok);
  }
  .bad span[aria-hidden] {
    color: var(--bad);
  }
  .muted span[aria-hidden] {
    color: var(--ink-3);
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  .stop {
    font-size: 18px;
    padding: 0.5rem 1rem;
  }
</style>
