<script>
  // The pacer's stroke as a bar that fills from the catch: the drive (legs,
  // swing, arms), then the recovery (hands away, body over, slide), each as
  // long as the rhythm to row at gives it and timed as the pace ghost rows
  // it. The step the pacer is in is lit, so the bar says what to do now; it
  // empties at each catch. Catch as it empties.
  import { stepsAt } from './pacer.js';

  let { rhythm, beat } = $props();

  const cycle = $derived(rhythm.drive_s + rhythm.recovery_s);
  const steps = $derived(stepsAt(rhythm));
  // How far through the stroke the pacer is, 0–1.
  const at = $derived(
    (beat.drive ? beat.u * rhythm.drive_s : rhythm.drive_s + beat.u * rhythm.recovery_s) / cycle,
  );
  const current = $derived(steps.findLast((s) => s.from <= at));
  const finish = $derived((rhythm.drive_s / cycle) * 100);
</script>

<div class="pacer">
  <div class="bar" role="img" aria-label="The stroke to row: {current.name} now">
    <div class="fill" class:drive={current.drive} style:width="{at * 100}%"></div>
    {#each steps as s (s.name)}
      <div
        class="step"
        class:drive={s.drive}
        class:now={s === current}
        style:left="{s.from * 100}%"
        style:width="{s.width * 100}%"
      >
        <span>{s.name}</span>
      </div>
    {/each}
  </div>
  <div class="ends" aria-hidden="true">
    <span style:left="0%">catch</span>
    <span style:left="{finish}%">finish</span>
    <span style:left="100%">catch</span>
  </div>
</div>

<style>
  .pacer {
    margin: 0 0 0.5rem;
  }
  .bar {
    position: relative;
    height: 48px;
    border-radius: 8px;
    background: var(--rule);
    overflow: hidden;
  }
  /* The stroke so far: accent through the drive, calmer through the recovery. */
  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--mean);
    opacity: 0.35;
  }
  .fill.drive {
    background: var(--accent);
    opacity: 0.4;
  }
  .step {
    position: absolute;
    top: 0;
    bottom: 0;
    display: grid;
    place-items: center;
    box-shadow: inset -2px 0 0 var(--surface);
    color: var(--ink-3);
    font-size: 16px;
    white-space: nowrap;
  }
  .step.drive {
    box-shadow:
      inset -2px 0 0 var(--surface),
      inset 0 -4px 0 var(--accent);
  }
  .step.now {
    color: var(--ink);
    font-size: 24px;
    font-weight: 600;
    background: color-mix(in srgb, var(--mean) 35%, transparent);
  }
  .step.drive.now {
    background: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .ends {
    position: relative;
    height: 1.2rem;
    margin-top: 0.2rem;
    font-size: 14px;
    color: var(--ink-3);
  }
  .ends span {
    position: absolute;
    transform: translateX(-50%);
  }
  .ends span:first-child {
    transform: none;
  }
  .ends span:last-child {
    transform: translateX(-100%);
  }
</style>
