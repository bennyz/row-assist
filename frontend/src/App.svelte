<script>
  import { listSessions } from './lib/api.js';
  import SessionList from './lib/SessionList.svelte';
  import SessionView from './lib/SessionView.svelte';
  import CameraPage from './lib/CameraPage.svelte';
  import MirrorPage from './lib/MirrorPage.svelte';
  import Readiness from './lib/Readiness.svelte';
  import ProgressPage from './lib/ProgressPage.svelte';
  import Today from './lib/Today.svelte';
  import PhonePage from './lib/PhonePage.svelte';

  let sessions = $state([]);
  let current = $state(null);
  let error = $state(null);
  let loaded = $state(false);
  let path = $state(location.pathname);
  const onCamera = $derived(path.startsWith('/camera'));
  // The row screen, in front of the erg, gets the whole window: it's home.
  const onRow = $derived(path === '/' || path.startsWith('/mirror'));
  const onProgress = $derived(path.startsWith('/progress'));
  // A phone gets its own full-window pages: setup, then the camera.
  const onPhoneCamera = $derived(path.startsWith('/phone/camera'));
  const onPhone = $derived(path.startsWith('/phone'));

  const fromPath = () => {
    const m = location.pathname.match(/^\/sessions\/(\d+)/);
    return m ? Number(m[1]) : null;
  };

  function load() {
    return listSessions()
      .then((list) => {
        sessions = list;
        current = fromPath() ?? list[0]?.id ?? null;
      })
      .catch((e) => (error = e.message))
      .finally(() => (loaded = true));
  }
  // The phone's pages can't see the sessions (only this computer can).
  if (!location.pathname.startsWith('/phone')) load();

  function go(to) {
    history.pushState(null, '', to);
    path = to;
  }

  // A row was saved: show its review, with the list brought up to date.
  function saved(id) {
    go(`/sessions/${id}`);
    load();
  }

  // On a review page, a new row starting brings the row screen back.
  $effect(() => {
    if (onRow || onPhone) return;
    let ws;
    let timer;
    let alive = true;
    const connect = () => {
      if (!alive) return;
      ws = new WebSocket(`${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws/live`);
      ws.onmessage = (e) => {
        const m = JSON.parse(e.data);
        if (m.type === 'row' && m.phase === 'rowing') go('/');
      };
      ws.onclose = () => {
        if (alive) timer = setTimeout(connect, 3000);
      };
    };
    connect();
    return () => {
      alive = false;
      clearTimeout(timer);
      ws?.close();
    };
  });

  function select(id) {
    current = id;
    go(`/sessions/${id}`);
  }

  window.addEventListener('popstate', () => {
    path = location.pathname;
    current = fromPath() ?? sessions[0]?.id ?? null;
  });
</script>

{#if onRow}
  <MirrorPage onsaved={saved} />
{:else if onPhoneCamera}
  <CameraPage phone />
{:else if onPhone}
  <PhonePage />
{:else}
<div class="layout">
  <aside>
    <div class="brand">Row Assist</div>
    <Readiness />
    <Today />
    <a
      class="nav"
      href="/"
      onclick={(e) => {
        e.preventDefault();
        go('/');
      }}>Row screen</a
    >
    <a
      class="nav"
      class:here={onProgress}
      href="/progress"
      onclick={(e) => {
        e.preventDefault();
        go('/progress');
      }}>Progress</a
    >
    <a
      class="nav"
      class:here={onCamera}
      href="/camera"
      onclick={(e) => {
        e.preventDefault();
        go('/camera');
      }}>Camera setup</a
    >
    {#if sessions.length}
      <SessionList {sessions} current={onCamera || onProgress ? null : current} onselect={select} />
    {/if}
  </aside>
  <main>
    {#if onCamera}
      <CameraPage />
    {:else if onProgress}
      <ProgressPage />
    {:else if error}
      <p class="error">Couldn't reach the server: {error}</p>
    {:else if current != null}
      <SessionView id={current} />
    {:else if loaded}
      <div class="empty">
        <p>No sessions yet.</p>
        <p class="muted">
          Open the <a href="/">row screen</a> and start rowing: it records from your first stroke.
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
