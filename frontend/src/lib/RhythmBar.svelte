<script>
  // One stroke at the target rhythm: the drive, then a recovery about twice
  // as long. The marker is where you are in this stroke; the bar under it is
  // your last stroke, by the camera. On rhythm, you reach the catch as the
  // marker reaches the end.
  let { rhythm, sinceCatch = null, last = null } = $props();

  const cycle = $derived(rhythm.drive_s + rhythm.recovery_s);
  const pct = (s) => Math.min(100, Math.max(0, (s / cycle) * 100));
  const s1 = (v) => `${v.toFixed(1)} s`;
  // Rushing the recovery is the fault; a long one at a low rate is fine.
  const verdict = $derived.by(() => {
    if (!last?.drive_s || !last?.recovery_s) return null;
    const short = rhythm.recovery_s - last.recovery_s;
    if (short > 0.25) return { tone: 'warn', text: `Slow the slide: ${short.toFixed(1)} s more on the way forward` };
    return { tone: 'ok', text: 'On rhythm' };
  });
  const overrun = $derived(last ? last.drive_s + last.recovery_s > cycle : false);
</script>

<div class="rhythm">
  <div class="labels">
    <span>Drive <strong class="num">{s1(rhythm.drive_s)}</strong></span>
    <span>Recovery <strong class="num">{s1(rhythm.recovery_s)}</strong></span>
    <span class="rate">at rate {rhythm.rate.toFixed(0)}</span>
  </div>
  <div class="track target" aria-hidden="true">
    <div class="drive" style:width="{pct(rhythm.drive_s)}%"></div>
    <div class="recovery"></div>
    {#if sinceCatch != null && sinceCatch < cycle * 1.5}<div class="now" style:left="{pct(sinceCatch)}%"></div>{/if}
  </div>
  {#if last?.drive_s && last?.recovery_s}
    <div class="track you" aria-hidden="true">
      <div class="drive" style:width="{pct(last.drive_s)}%"></div>
      <div class="recovery" style:width="{pct(last.recovery_s)}%" class:overrun></div>
    </div>
    <p class="verdict" data-tone={verdict?.tone}>
      Last stroke: drive {s1(last.drive_s)} · recovery {s1(last.recovery_s)}{#if verdict}{' · '}<strong>{verdict.text}</strong>{/if}
    </p>
  {/if}
</div>

<style>
  .rhythm {
    margin-top: 1rem;
  }
  .labels {
    display: flex;
    gap: 1.5rem;
    font-size: 18px;
    color: var(--ink-3);
  }
  .labels strong {
    color: var(--ink);
    font-weight: 600;
  }
  .rate {
    margin-left: auto;
  }
  .track {
    position: relative;
    display: flex;
    gap: 2px;
    margin-top: 0.5rem;
  }
  .target {
    height: 22px;
  }
  .you {
    height: 10px;
  }
  .drive,
  .recovery {
    height: 100%;
    border-radius: 4px;
  }
  .target .drive {
    background: var(--accent);
  }
  .target .recovery {
    flex: 1;
    background: var(--accent-wash);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .you .drive {
    background: var(--ink-2);
  }
  .you .recovery {
    background: var(--ink-3);
  }
  .you .recovery.overrun {
    border-radius: 4px 0 0 4px;
  }
  .now {
    position: absolute;
    top: -6px;
    bottom: -6px;
    width: 4px;
    margin-left: -2px;
    border-radius: 2px;
    background: var(--ink);
    box-shadow: 0 0 0 2px var(--surface);
  }
  .verdict {
    margin: 0.5rem 0 0;
    font-size: 18px;
    color: var(--ink-2);
  }
  .verdict strong {
    color: var(--ink);
  }
  .verdict[data-tone='warn'] strong {
    color: var(--warn);
  }
</style>
