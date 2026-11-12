<script>
  // The order one stroke was rowed in: for each part, where in the handle's
  // travel it did its moving, over where the good stroke does it. Rows are
  // in the order the parts should go, so a good stroke steps down like
  // stairs in both halves: legs, body, arms on the drive; arms, body, legs
  // on the way forward. A part that went before its turn is marked (the
  // server judges that: `faults` are the stroke's fault keys).
  import { FAULTS, PARTS } from './format.js';

  let { sequence, target, faults = [] } = $props();

  const PHASES = [
    { key: 'drive', title: 'Drive', from: 'catch', to: 'finish', order: ['legs', 'body', 'arms'] },
    { key: 'recovery', title: 'Recovery', from: 'finish', to: 'catch', order: ['arms', 'body', 'legs'] },
  ];
  const early = $derived(new Set(faults.map((f) => FAULTS[f]?.at?.join('.')).filter(Boolean)));
  const pct = (v) => `${(v * 100).toFixed(1)}%`;
  const rows = $derived(
    PHASES.map((p) => ({
      ...p,
      parts: p.order.map((part) => ({
        part,
        label: p.key === 'recovery' && part === 'legs' ? 'Slide' : PARTS[part],
        mine: sequence?.[p.key]?.[part] ?? null,
        target: target?.[p.key]?.[part] ?? null,
        early: early.has(`${p.key}.${part}`),
      })),
    })),
  );
</script>

<div class="sequence">
  {#each rows as p (p.key)}
    <div class="phase" role="img" aria-label="{p.title}: when the legs, the body and the arms moved, against the good stroke">
      <div class="head">
        <strong>{p.title}</strong>
        <span>{p.from} → {p.to}</span>
      </div>
      {#each p.parts as r (r.part)}
        <div class="row" class:early={r.early}>
          <span class="label">{r.label}</span>
          <span class="track">
            {#if r.target}
              <span class="target" style:left={pct(r.target.start)} style:width={pct(r.target.end - r.target.start)}></span>
            {/if}
            {#if r.mine}
              <span class="mine" style:left={pct(r.mine.start)} style:width={pct(Math.max(r.mine.end - r.mine.start, 0.02))}></span>
            {/if}
          </span>
          <span class="verdict">{r.early ? 'early' : ''}</span>
        </div>
      {/each}
    </div>
  {/each}
  <p class="legend">
    <span class="key you"></span>you <span class="key good"></span>good stroke
  </p>
</div>

<style>
  .sequence {
    display: grid;
    gap: 0.9rem;
    max-width: 34rem;
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 0.6rem;
    margin-bottom: 0.3rem;
    font-size: 13px;
    color: var(--ink-3);
  }
  .head strong {
    font-size: 14px;
    font-weight: 600;
    color: var(--ink-2);
  }
  .row {
    display: grid;
    grid-template-columns: 3.4rem 1fr 2.6rem;
    align-items: center;
    gap: 0.6rem;
    height: 1.35rem;
    font-size: 14px;
    color: var(--ink-2);
  }
  .track {
    position: relative;
    height: 100%;
    border-left: 1px solid var(--rule);
    border-right: 1px solid var(--rule);
  }
  .target,
  .mine {
    position: absolute;
    border-radius: 2px;
  }
  .target {
    top: 2px;
    bottom: 2px;
    background: var(--mean);
    opacity: 0.4;
  }
  .mine {
    top: 7px;
    bottom: 7px;
    background: var(--accent);
  }
  .row.early .mine {
    background: var(--bad);
  }
  .row.early .label {
    color: var(--ink);
  }
  .verdict {
    font-size: 13px;
    color: var(--bad);
  }
  .legend {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0;
    font-size: 12px;
    color: var(--ink-3);
  }
  .key {
    display: inline-block;
    width: 1.1rem;
    border-radius: 2px;
  }
  .key.you {
    height: 5px;
    background: var(--accent);
  }
  .key.good {
    height: 10px;
    margin-left: 0.8rem;
    background: var(--mean);
    opacity: 0.4;
  }
</style>
