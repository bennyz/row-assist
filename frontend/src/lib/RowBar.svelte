<script>
  // The row's state and the devices, across the top of the row screen. Read
  // from the erg: one line, big. Everything comes from the server's `row`
  // message; the countdown is only display.
  import { clock } from './format.js';

  let { row, minutes = null, onstop } = $props();

  // A rest shorter than this isn't worth a countdown.
  const COUNTDOWN_AFTER_S = 15;

  const ending = $derived(
    row.phase === 'rowing' && row.idle_s != null && row.idle_s >= COUNTDOWN_AFTER_S
      ? Math.max(0, row.ends_after_s - row.idle_s)
      : null,
  );

  const devices = $derived([
    { name: 'PM5', link: row.devices.pm5 },
    { name: row.hr != null && row.devices.h10.connected ? `Heart ${row.hr}` : 'Heart', link: row.devices.h10 },
    { name: 'Camera', link: row.devices.camera },
  ]);
  const mark = (c) => (c === true ? '✓' : c === false ? '✕' : '…');
  const tone = (c) => (c === true ? 'ok' : c === false ? 'bad' : 'muted');
</script>

<div class="bar">
  <p class="what" aria-live="polite">
    {#if row.phase === 'ready'}
      <strong>Ready.</strong> Start rowing: it records from your first stroke.
    {:else if row.phase === 'rowing'}
      <strong class="rec"><span aria-hidden="true">●</span> Recording</strong>
      <span class="num">{clock(row.elapsed_s)}</span>{#if minutes}{' '}<span class="of">of {clock(minutes * 60)}</span>{/if} · {row.strokes} strokes
      {#if ending != null}<span class="ending">· saves in {clock(ending)} unless you row</span>{/if}
    {:else}
      <strong>Saving…</strong>
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
  {#if row.phase === 'rowing'}
    <button class="stop" onclick={onstop}>Stop and save</button>
  {/if}
</div>

<style>
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
  .of {
    font-family: var(--font-num);
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
