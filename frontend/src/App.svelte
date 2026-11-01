<script>
  import { listSessions } from './lib/api.js';
  import SessionList from './lib/SessionList.svelte';
  import SessionView from './lib/SessionView.svelte';

  let sessions = $state([]);
  let current = $state(null);
  let error = $state(null);
  let loaded = $state(false);

  const fromPath = () => {
    const m = location.pathname.match(/^\/sessions\/(\d+)/);
    return m ? Number(m[1]) : null;
  };

  listSessions()
    .then((list) => {
      sessions = list;
      current = fromPath() ?? list[0]?.id ?? null;
    })
    .catch((e) => (error = e.message))
    .finally(() => (loaded = true));

  function select(id) {
    current = id;
    history.pushState(null, '', `/sessions/${id}`);
  }

  window.addEventListener('popstate', () => (current = fromPath() ?? sessions[0]?.id ?? null));
</script>

<div class="layout">
  <aside>
    <div class="brand">Row Assist</div>
    {#if sessions.length}
      <SessionList {sessions} {current} onselect={select} />
    {/if}
  </aside>
  <main>
    {#if error}
      <p class="error">Couldn't reach the server: {error}</p>
    {:else if current != null}
      <SessionView id={current} />
    {:else if loaded}
      <div class="empty">
        <p>No sessions yet.</p>
        <p class="muted">
          Record one with <code>rowctl record --plan reference --speak</code>, or try it without the
          erg: <code>rowctl simulate --out /tmp/ref.rowrec</code> then
          <code>rowctl analyze /tmp/ref.rowrec</code>.
        </p>
      </div>
    {/if}
  </main>
</div>

<style>
  .layout {
    display: grid;
    grid-template-columns: 15rem minmax(0, 1fr);
    min-height: 100vh;
  }
  aside {
    background: var(--bg);
    border-right: 1px solid var(--rule);
    padding: 1.25rem 0 1.25rem 1rem;
  }
  .brand {
    font: 600 15px var(--font-num);
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--accent);
    margin-bottom: 1.75rem;
  }
  main {
    background: var(--surface);
    padding: 1.5rem 2.25rem 4rem;
    min-width: 0;
  }
  code {
    font-family: var(--font-num);
    color: var(--ink);
  }
  .error {
    color: var(--z4);
  }
  @media (max-width: 760px) {
    .layout {
      grid-template-columns: minmax(0, 1fr);
    }
    aside {
      border-right: 0;
      border-bottom: 1px solid var(--rule);
      padding: 1rem;
    }
    main {
      padding: 1rem 1rem 3rem;
    }
  }
</style>
