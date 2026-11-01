<script>
  import { dateLabel, fixed, timeLabel } from './format.js';

  let { sessions, current, onselect } = $props();
</script>

<nav aria-label="Sessions">
  <h2>Sessions</h2>
  <ol>
    {#each sessions as s (s.id)}
      <li>
        <a
          href="/sessions/{s.id}"
          class:current={s.id === current}
          aria-current={s.id === current ? 'page' : undefined}
          onclick={(e) => {
            e.preventDefault();
            onselect(s.id);
          }}
        >
          <span class="when">{dateLabel(s.started_at_us)} <span class="muted">{timeLabel(s.started_at_us)}</span></span>
          <span class="facts num">
            {fixed(s.minutes)} min · {fixed(s.watts)} W · EF {fixed(s.ef, 3)}
          </span>
          {#if s.plan || s.simulated}
            <span class="tags">{[s.plan, s.simulated && 'sim'].filter(Boolean).join(' · ')}</span>
          {/if}
        </a>
      </li>
    {/each}
  </ol>
</nav>

<style>
  h2 {
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-3);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 0 0 0.75rem;
  }
  ol {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  a {
    display: grid;
    gap: 0.1rem;
    padding: 0.6rem 0.75rem;
    border-left: 2px solid transparent;
    color: var(--ink-2);
    text-decoration: none;
    border-bottom: 1px solid var(--rule);
  }
  a:hover {
    color: var(--ink);
  }
  a.current {
    border-left-color: var(--accent);
    color: var(--ink);
    background: var(--surface);
  }
  .when {
    font-size: 14px;
    font-weight: 500;
  }
  .facts {
    font-size: 13px;
  }
  .tags {
    font-size: 12px;
    color: var(--accent);
  }
</style>
