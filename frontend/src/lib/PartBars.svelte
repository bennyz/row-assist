<script>
  // Where the legs, the body and the arms are right now, each from its catch
  // position (left) to its finish position (right), beside where the good
  // stroke has it with the handle where yours is (the tick). A part away
  // from its tick is named with which way: early (it went before its turn)
  // or late. `off` is held a moment by the page, so a word can be read.
  import { PARTS } from './format.js';

  let { guide, off = {} } = $props();

  const pct = (v) => `${Math.min(100, Math.max(0, v * 100)).toFixed(1)}%`;
  const rows = $derived(
    Object.keys(PARTS).map((part) => ({
      part,
      label: PARTS[part],
      you: guide.you[part],
      ideal: guide.ideal[part],
      off: off[part] ?? null,
    })),
  );
</script>

<div class="parts">
  {#each rows as r (r.part)}
    <div class="row" class:off={r.off}>
      <span class="label">{r.label}</span>
      <span class="track" aria-hidden="true">
        <span class="you" style:width={pct(r.you)}></span>
        <span class="ideal" style:left={pct(r.ideal)}></span>
      </span>
      <span class="word" aria-live="off">{r.off ?? ''}</span>
    </div>
  {/each}
  <p class="ends"><span>catch</span><span>finish</span></p>
</div>

<style>
  .parts {
    display: grid;
    gap: 0.55rem;
  }
  .row {
    display: grid;
    grid-template-columns: 4.2rem 1fr 3.4rem;
    align-items: center;
    gap: 0.75rem;
    font-size: 20px;
    color: var(--ink-2);
  }
  .track {
    position: relative;
    height: 16px;
    border-radius: 3px;
    background: var(--rule);
  }
  .you {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 3px;
    background: var(--accent);
  }
  .ideal {
    position: absolute;
    top: -5px;
    bottom: -5px;
    width: 4px;
    margin-left: -2px;
    border-radius: 2px;
    background: var(--ink);
    box-shadow: 0 0 0 2px var(--surface);
  }
  .row.off .label {
    color: var(--ink);
  }
  .row.off .you {
    background: var(--bad);
  }
  .word {
    font-size: 18px;
    color: var(--bad);
  }
  .ends {
    display: flex;
    justify-content: space-between;
    margin: 0 4.15rem 0 4.95rem;
    font-size: 12px;
    color: var(--ink-3);
  }
</style>
