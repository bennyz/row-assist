<script>
  import { listSessions } from './lib/api.js';
  import SessionList from './lib/SessionList.svelte';
  import SessionView from './lib/SessionView.svelte';
  import CameraPage from './lib/CameraPage.svelte';
  import MirrorPage from './lib/MirrorPage.svelte';
  import Readiness from './lib/Readiness.svelte';

  let sessions = $state([]);
  let current = $state(null);
  let error = $state(null);
  let loaded = $state(false);
  let path = $state(location.pathname);
  const onCamera = $derived(path.startsWith('/camera'));
  // The front screen gets the whole window.
  const onMirror = $derived(path.startsWith('/mirror'));

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

  function go(to) {
    history.pushState(null, '', to);
    path = to;
  }

  function select(id) {
    current = id;
    go(`/sessions/${id}`);
  }

  window.addEventListener('popstate', () => {
    path = location.pathname;
    current = fromPath() ?? sessions[0]?.id ?? null;
  });
</script>

{#if onMirror}
  <MirrorPage />
{:else}
<div class="layout">
  <aside>
    <div class="brand">Row Assist</div>
    <Readiness />
    <a
      class="nav"
      class:here={onCamera}
      href="/camera"
      onclick={(e) => {
        e.preventDefault();
        go('/camera');
      }}>Camera setup</a
    >
    <a class="nav" href="/mirror" target="_blank" rel="noopener">Mirror (front screen) ↗</a>
    {#if sessions.length}
      <SessionList {sessions} current={onCamera ? null : current} onselect={select} />
    {/if}
  </aside>
  <main>
    {#if onCamera}
      <CameraPage />
    {:else if error}
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
{/if}


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
  .nav {
    display: block;
    margin: 0 0 0.4rem;
    padding: 0.35rem 0.75rem;
    border-left: 2px solid transparent;
    color: var(--ink-2);
    text-decoration: none;
    font-size: 14px;
  }
  .nav:hover {
    color: var(--ink);
  }
  .nav:last-of-type {
    margin-bottom: 1.75rem;
  }
  .nav.here {
    border-left-color: var(--accent);
    color: var(--ink);
    background: var(--surface);
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
    color: var(--bad);
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
