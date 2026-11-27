<script>
  // The pacer boat, under the figure: it rows the target power from Go, and
  // the water runs by at its pace. Your boat, in the lane beside it, sits
  // ahead or behind by your speed over your last few strokes (`lead`, in
  // power bands: 0 level, ±1 at the band's edges; null when you aren't
  // rowing). Keep level with it and the power is right. Ahead is no lead to
  // keep: it comes back level as soon as you row at its pace. The words say
  // what to do: "Ahead: lighter" was read as a riddle until well into a row.
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
      ? 'Ahead: ease off'
      : where === 'behind'
        ? harder
          ? 'Behind: pull harder'
          : 'Behind'
        : where === 'with'
          ? 'Level: hold this'
          : 'Keep level with the pacer',
  );
  const flow = $derived(((now / 1000) * speed * FLOW_PX) % MARKS_PX);
</script>

<div class="pace" data-where={where}>
  <div class="lane" role="img" aria-label="The pacer boat at {Math.round(watts)} watts: {say}">
    <div class="water" style:background-position-x="{-flow}px"></div>
    <div class="zone" style:left="{at(-1)}%" style:right="{100 - at(1)}%"></div>
    <div class="boat pacer" style:left="50%">Pacer {Math.round(watts)} W</div>
    {#if lead != null}<div class="boat you" style:left="{at(lead)}%">You</div>{/if}
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
  /* Two lanes, the pacer's above yours: level is one boat over the other,
     and neither hides the other's name. */
  .boat {
    position: absolute;
    height: 18px;
    width: 92px;
    transform: translateX(-50%);
    /* A bow to the right: the way the boats go. */
    border-radius: 9px 18px 18px 9px;
    display: grid;
    place-items: center;
    font: 600 13px var(--font-num);
    white-space: nowrap;
  }
  .pacer {
    top: 3px;
    background: color-mix(in srgb, var(--mean) 70%, transparent);
    color: var(--surface);
  }
  .you {
    bottom: 3px;
    background: var(--accent);
    color: var(--surface);
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
