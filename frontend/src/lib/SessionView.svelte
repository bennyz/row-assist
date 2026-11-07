<script>
  import { getSession } from './api.js';
  import { clock, dateLabel, faultList, fixed, grouped, pace, timeLabel } from './format.js';
  import FaultRates from './FaultRates.svelte';
  import Feel from './Feel.svelte';
  import Figures from './Figures.svelte';
  import ForceCurve from './ForceCurve.svelte';
  import StrokeCharts from './StrokeCharts.svelte';
  import StrokeTable from './StrokeTable.svelte';
  import ZoneBar from './ZoneBar.svelte';

  let { id } = $props();

  let detail = $state(null);
  let error = $state(null);
  let loading = $state(false);
  let selected = $state(null);

  $effect(() => {
    const want = id;
    loading = true;
    error = null;
    getSession(want)
      .then((d) => {
        if (want !== id) return;
        detail = d;
        selected = d.strongest ?? d.strokes[0]?.index ?? null;
      })
      .catch((e) => (error = e.message))
      .finally(() => (loading = false));
  });

  const s = $derived(detail?.summary);
  const w = $derived(s?.whole);
  const stroke = $derived(detail?.strokes.find((x) => x.index === selected));

  const readout = $derived(
    w
      ? [
          { label: 'time', value: clock(w.seconds) },
          { label: 'metres', value: grouped(w.distance_m) },
          { label: '/500m', value: pace(w.watts) },
          { label: 'watts', value: fixed(w.watts) },
          { label: 'spm', value: fixed(w.spm, 1) },
          { label: 'avg HR', value: fixed(w.hr) },
          { label: 'EF  W/bpm', value: fixed(w.ef, 3) },
        ]
      : [],
  );

  const secondary = $derived(
    s
      ? [
          ['decoupling', s.decoupling_pct == null ? null : `${fixed(s.decoupling_pct, 1)}%`],
          ['watts at 20', s.rate_20 ? `${fixed(s.rate_20.watts)} W` : null],
          ['last 10 min EF', s.last_10_min ? fixed(s.last_10_min.ef, 3) : null],
          ['TRIMP', s.heart?.trimp == null ? null : fixed(s.heart.trimp)],
          ['peak HR 10 s', s.heart?.peak_10s_bpm == null ? null : fixed(s.heart.peak_10s_bpm)],
          ['drag factor', s.drag_factor],
          ['drive', s.drive_ms == null ? null : `${fixed(s.drive_ms / 1000, 2)} s`],
          ['ratio', s.ratio == null ? null : fixed(s.ratio, 1)],
        ].filter(([, v]) => v != null)
      : [],
  );

  function pick(index) {
    if (index != null) selected = index;
  }
</script>

{#if error}
  <p class="error">Couldn't load session {id}: {error}</p>
{:else if detail}
  <article class:stale={loading}>
    <header>
      <h1>{dateLabel(detail.started_at_us)} <span class="muted">{timeLabel(detail.started_at_us)}</span></h1>
      <div class="tags">
        {#if detail.plan}<span class="tag">{detail.plan}</span>{/if}
        {#if detail.simulated}<span class="tag">simulated</span>{/if}
        {#if detail.note}<span class="note">{detail.note}</span>{/if}
      </div>
    </header>

    {#if !detail.simulated}
      <Feel id={detail.id} effort={detail.effort} comment={detail.comment} />
    {/if}

    <dl class="readout">
      {#each readout as r (r.label)}
        <div><dd class="num">{r.value}</dd><dt>{r.label}</dt></div>
      {/each}
    </dl>

    {#if secondary.length}
      <dl class="secondary">
        {#each secondary as [k, v] (k)}
          <div><dt>{k}</dt><dd class="num">{v}</dd></div>
        {/each}
      </dl>
    {/if}

    {#if detail.reference}
      {@const r = detail.reference}
      <p class="reference">
        {#if r.high != null}
          EF <strong class="num">{fixed(r.ef, 3)}</strong> vs {r.pieces} past steady pieces at
          {fixed(r.watts_band[0])}–{fixed(r.watts_band[1])} W: best {fixed(r.high, 3)}, worst {fixed(r.low, 3)}{#if r.ramp}{' '}<span class="ramp">→ {r.ramp}</span>{/if}
        {:else}
          <span class="muted">{r.note}</span>
        {/if}
      </p>
    {/if}

    {#if s.heart?.zone_s?.length}
      <section>
        <h2>Heart-rate zones</h2>
        <ZoneBar seconds={s.heart.zone_s} zones={detail.zones} />
      </section>
    {/if}

    <section>
      <h2>Through the session</h2>
      <StrokeCharts strokes={detail.strokes} zones={detail.zones} bind:selected />
    </section>

    <section>
      <div class="curve-head">
        <h2>Force curve</h2>
        <div class="controls">
          <button onclick={() => pick(detail.strongest)} disabled={detail.strongest == null}>Most work</button>
          <button onclick={() => pick(detail.weakest)} disabled={detail.weakest == null}>Least work</button>
        </div>
      </div>
      <div class="curve">
        <ForceCurve {stroke} mean={detail.mean_curve} />
        {#if stroke}
          <dl class="stroke-facts">
            <div><dt>stroke</dt><dd class="num">{stroke.index + 1}</dd></div>
            <div><dt>at</dt><dd class="num">{clock(stroke.t_s)}</dd></div>
            <div><dt>work</dt><dd class="num">{fixed(stroke.joules)} J</dd></div>
            <div><dt>power</dt><dd class="num">{fixed(stroke.watts)} W</dd></div>
            <div><dt>rate</dt><dd class="num">{fixed(stroke.rate, 1)}</dd></div>
            <div><dt>drive</dt><dd class="num">{fixed(stroke.drive_ms == null ? null : stroke.drive_ms / 1000, 2)} s</dd></div>
            <div><dt>peak</dt><dd class="num">{fixed(stroke.peak_n)} N</dd></div>
            <div><dt>HR</dt><dd class="num">{fixed(stroke.hr)}</dd></div>
            <div><dt>faults</dt><dd>{faultList(stroke.faults) || 'none'}</dd></div>
          </dl>
        {/if}
      </div>
    </section>

    {#if s.faults?.length}
      <section>
        <h2>Technique</h2>
        <FaultRates rates={s.faults} camera={!!s.camera} />
        {#if s.camera}
          <div class="figures">
            <Figures {stroke} typicalCatch={detail.typical_catch} typicalFinish={detail.typical_finish} />
          </div>
          <p class="muted cam">
            Camera: {s.camera.side} side{s.camera.offset_ms != null
              ? `, synced to the PM5 (${fixed(s.camera.offset_ms)} ms)`
              : ', not synced'}{Math.abs(s.camera.tilt_deg) >= 0.5 ? `, levelled ${fixed(s.camera.tilt_deg, 1)}°` : ''}
          </p>
        {/if}
      </section>
    {/if}

    <section>
      <StrokeTable strokes={detail.strokes} bind:selected />
    </section>
  </article>
{:else}
  <p class="muted">Loading…</p>
{/if}

<style>
  article {
    transition: opacity 0.15s;
  }
  article.stale {
    opacity: 0.6;
  }
  header {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 0.5rem 1rem;
  }
  h1 {
    font-size: 20px;
    font-weight: 600;
    margin: 0;
  }
  h2 {
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-3);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 0 0 0.75rem;
  }
  .tags {
    display: flex;
    gap: 0.5rem;
    align-items: baseline;
  }
  .tag {
    font-size: 12px;
    color: var(--accent);
    border: 1px solid var(--rule);
    border-radius: 3px;
    padding: 0 0.4rem;
  }
  .note {
    color: var(--ink-2);
    font-size: 14px;
  }
  dl {
    margin: 0;
  }
  dd {
    margin: 0;
  }
  .readout {
    display: flex;
    flex-wrap: wrap;
    margin: 1.5rem 0 0;
    border-top: 1px solid var(--rule);
    border-bottom: 1px solid var(--rule);
  }
  .readout > div {
    padding: 0.9rem 1.6rem 0.8rem 0;
    margin-right: 1.6rem;
    border-right: 1px solid var(--rule);
  }
  .readout > div:last-child {
    border-right: 0;
  }
  .readout dd {
    font-size: 40px;
    font-weight: 600;
    line-height: 1;
    letter-spacing: -0.01em;
  }
  .readout dt {
    margin-top: 0.4rem;
    font-size: 12px;
    color: var(--ink-3);
  }
  .secondary {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 1.75rem;
    margin: 0.9rem 0 0;
    font-size: 14px;
  }
  .secondary div {
    display: flex;
    gap: 0.45rem;
    align-items: baseline;
  }
  .secondary dt {
    color: var(--ink-3);
  }
  .secondary dd {
    color: var(--ink);
    font-size: 15px;
  }
  section {
    margin-top: 2.25rem;
  }
  .curve-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }
  .controls {
    display: flex;
    gap: 0.5rem;
  }
  .curve {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 11rem;
    gap: 2rem;
    align-items: start;
  }
  .stroke-facts {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.35rem 1rem;
    margin-top: 1.6rem;
    font-size: 14px;
  }
  .stroke-facts div {
    display: contents;
  }
  .stroke-facts dt {
    color: var(--ink-3);
  }
  .stroke-facts dd {
    text-align: right;
  }
  .error {
    color: var(--bad);
  }
  .figures {
    margin-top: 1.75rem;
  }
  .reference {
    margin: 0.9rem 0 0;
    font-size: 14px;
    color: var(--ink-2);
  }
  .reference strong {
    color: var(--ink);
  }
  .ramp {
    color: var(--ink);
  }
  .cam {
    font-size: 13px;
    margin: 1rem 0 0;
  }
  @media (max-width: 760px) {
    .curve {
      grid-template-columns: minmax(0, 1fr);
    }
    .readout dd {
      font-size: 30px;
    }
  }
</style>
