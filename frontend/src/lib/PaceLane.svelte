<script>
  // The pacer boat, under the figure: it rows the target power from Go, and
  // the water runs by at its pace. Your boat sits ahead of it or behind by
  // your speed over your last few strokes (`lead`, in power bands: 0
  // alongside, ±1 at the band's edges; null when you aren't rowing). Keep
  // alongside it and the power is right. Ahead is no lead to keep: it comes
  // back alongside as soon as you row at its pace.
  let { lead, watts, speed, now, harder = true } = $props();

  // Power bands either side of the pacer the lane shows.
  const SPAN = 3;
  // Half the lane's width a full SPAN takes, %.
  const REACH = 42;
  // The water's marks, px apart (as in .water), and how fast they run per
  // m/s of the pacer.
  const MARKS_PX = 48;
  const FLOW_PX = 40;

  const at = (u) => 50 + (Math.max(-SPAN, Math.min(SPAN, u)) / SPAN) * REACH;
  const where = $derived(lead == null ? null : lead > 1 ? 'ahead' : lead < -1 ? 'behind' : 'with');
  const say = $derived(
    where === 'ahead'
      ? 'Ahead: lighter'
      : where === 'behind'
        ? harder
          ? 'Behind: harder'
          : 'Behind'
        : where === 'with'
          ? 'With the pacer'
          : 'Row with the pacer',
  );
  const flow = $derived(((now / 1000) * speed * FLOW_PX) % MARKS_PX);
</script>

<div class="pace" data-where={where}>
  <div class="lane" role="img" aria-label="The pacer boat at {Math.round(watts)} watts: {say}">
    <div class="water" style:background-position-x="{-flow}px"></div>
    <div class="zone" style:left="{at(-1)}%" style:right="{100 - at(1)}%"></div>
    <div class="boat pacer" style:left="50%"><span>{Math.round(watts)} W</span></div>
    {#if lead != null}<div class="boat you" style:left="{at(lead)}%"></div>{/if}
  </div>
  <p class="say" aria-live="polite">{say}</p>
</div>

<style>
  .pace {
    display: flex;
    align-items: center;
    gap: 1.5rem;
    margin: 0 0 0.6rem;
  }
  .lane {
    position: relative;
    flex: 1;
    height: 44px;
    border-radius: 8px;
    background: var(--rule);
    overflow: hidden;
  }
  /* The water going by, a mark every MARKS_PX: the pacer is moving even
     when you aren't. */
  .water {
    position: absolute;
    inset: 0;
    background-image: repeating-linear-gradient(
      90deg,
      transparent 0 45px,
      color-mix(in srgb, var(--ink-3) 35%, transparent) 45px 48px
    );
  }
  /* Alongside: within the power band. */
  .zone {
    position: absolute;
    top: 0;
    bottom: 0;
    background: var(--accent-wash);
    box-shadow:
      inset 1px 0 0 var(--accent),
      inset -1px 0 0 var(--accent);
  }
  .boat {
    position: absolute;
    top: 50%;
    height: 22px;
    width: 76px;
    transform: translate(-50%, -50%);
    /* A bow to the right: the way the boats go. */
    border-radius: 11px 22px 22px 11px;
    display: grid;
    place-items: center;
  }
  .pacer {
    background: color-mix(in srgb, var(--mean) 70%, transparent);
    color: var(--surface);
    font: 600 14px var(--font-num);
  }
  .you {
    height: 14px;
    width: 64px;
    background: var(--accent);
    box-shadow: 0 0 0 2px var(--surface);
    transition: left 0.6s ease-out;
  }
  .say {
    min-width: 15rem;
    margin: 0;
    font: 600 26px var(--font-num);
    color: var(--ink-2);
  }
  [data-where='with'] .say {
    color: var(--ok);
  }
  [data-where='ahead'] .say,
  [data-where='behind'] .say {
    color: var(--warn);
  }
</style>
