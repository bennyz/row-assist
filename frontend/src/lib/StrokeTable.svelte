<script>
  // Every stroke as a table: the charts' values without hovering.
  import { clock, faultList, fixed } from './format.js';

  let { strokes, selected = $bindable() } = $props();
</script>

<details>
  <summary>All strokes <span class="muted">({strokes.length})</span></summary>
  <div class="scroll">
    <table class="tab">
      <thead>
        <tr>
          <th>#</th><th>time</th><th>drive s</th><th>ratio</th><th>W</th><th>spm</th><th>J</th><th>m</th>
          <th>HR</th><th>peak N</th><th class="left">faults</th>
        </tr>
      </thead>
      <tbody>
        {#each strokes as s (s.index)}
          <tr class:sel={s.index === selected} onclick={() => (selected = s.index)}>
            <td>{s.index + 1}</td>
            <td>{clock(s.t_s)}</td>
            <td>{fixed(s.drive_ms == null ? null : s.drive_ms / 1000, 2)}</td>
            <td>{fixed(s.ratio, 1)}</td>
            <td>{fixed(s.watts)}</td>
            <td>{fixed(s.rate, 1)}</td>
            <td>{fixed(s.joules)}</td>
            <td>{fixed(s.distance_m, 1)}</td>
            <td>{fixed(s.hr)}</td>
            <td>{fixed(s.peak_n)}</td>
            <td class="left">{faultList(s.faults, 'short')}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</details>

<style>
  summary {
    cursor: pointer;
    color: var(--ink-2);
    font-size: 14px;
  }
  .scroll {
    max-height: 22rem;
    overflow: auto;
    margin-top: 0.5rem;
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: 13px;
  }
  th {
    position: sticky;
    top: 0;
    background: var(--surface);
    color: var(--ink-3);
    font-weight: 500;
    text-align: right;
    padding: 0.3rem 0.6rem;
    border-bottom: 1px solid var(--rule);
  }
  td {
    text-align: right;
    padding: 0.2rem 0.6rem;
    color: var(--ink-2);
    border-bottom: 1px solid var(--rule);
  }
  tr {
    cursor: pointer;
  }
  .left {
    text-align: left;
  }
  tbody tr:hover td {
    color: var(--ink);
  }
  tr.sel td {
    color: var(--ink);
    box-shadow: inset 0 -1px var(--accent);
  }
  tr.sel td:first-child {
    box-shadow: inset 2px 0 var(--accent), inset 0 -1px var(--accent);
  }
</style>
