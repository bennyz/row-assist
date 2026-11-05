<script>
  // One-time setup for a phone as the side camera: trust Row Assist's
  // certificate (browsers only open the camera over HTTPS), then open the
  // camera page. Served over plain HTTP, so it works before that.
  import { onDestroy, onMount } from 'svelte';
  import { getPhone } from './api.js';

  let info = $state(null);
  let error = $state(null);
  let trusted = $state(null);
  const ios = /iPhone|iPad|iPod/.test(navigator.userAgent);

  // A no-cors request to the HTTPS address succeeds only once the phone
  // trusts the certificate.
  async function check() {
    if (!info) return;
    trusted = null;
    try {
      await fetch(`${info.origin}/api/phone`, { mode: 'no-cors', cache: 'no-store' });
      trusted = true;
    } catch {
      trusted = false;
    }
  }

  // Back from Settings: check again.
  const onVisible = () => document.visibilityState === 'visible' && check();
  onMount(() => document.addEventListener('visibilitychange', onVisible));
  onDestroy(() => document.removeEventListener('visibilitychange', onVisible));

  getPhone()
    .then((i) => {
      info = i;
      check();
    })
    .catch((e) => (error = e.message));
</script>

<div class="phone">
  <div class="brand">Row Assist</div>
  <h1>Phone camera</h1>
  {#if error}
    <p class="bad">No phone camera here ({error}). Start Row Assist with <code>rowctl record --phone</code> (or <code>watch --phone</code>) and scan the code it prints.</p>
  {:else if !info}
    <p class="muted">Loading…</p>
  {:else if trusted}
    <p class="ok"><span aria-hidden="true">✓</span> This phone trusts Row Assist.</p>
    <a class="go" href={info.camera_url}>Open the camera</a>
    <p class="muted">Bookmark it for next time: it stays the same while the computer keeps its network address.</p>
  {:else}
    <p>
      {trusted === null ? 'Checking the certificate…' : "The phone needs to trust Row Assist's certificate once before it can use the camera."}
    </p>
    {#if trusted === false}
      {#if ios}
        <ol class="steps">
          <li>
            <a class="go" href={info.ca_url}>Download the certificate</a>
            <span class="muted">Safari asks to allow a configuration profile: Allow.</span>
          </li>
          <li>Open Settings and tap <b>Profile Downloaded</b> at the top (or General → VPN &amp; Device Management), then Install.</li>
          <li>Settings → General → About → Certificate Trust Settings: turn on <b>Row Assist local CA</b>.</li>
          <li>Come back here. <button onclick={check}>Check again</button></li>
        </ol>
      {:else}
        <ol class="steps">
          <li><a class="go" href={info.ca_url}>Download the certificate</a></li>
          <li>Android: Settings → Security → Encryption &amp; credentials → Install a certificate → CA certificate, and pick the download.</li>
          <li>Come back here. <button onclick={check}>Check again</button></li>
        </ol>
      {/if}
      <p class="muted small">
        The certificate is made on your computer, and the key that signs with it never leaves it. To undo, remove the
        profile in Settings → General → VPN &amp; Device Management.
      </p>
    {/if}
  {/if}

  <h2>At the erg</h2>
  <ul class="muted">
    <li>Landscape, side-on to the erg, lens about hip height, 2–3 m away, plugged in.</li>
    <li>On the camera page, pick the Back Camera, not Ultra Wide (it bends the edges).</li>
    <li>The setup check also shows on the mirror in front of you.</li>
  </ul>
</div>

<style>
  .phone {
    max-width: 36rem;
    margin: 0 auto;
    padding: 1.25rem 1rem 3rem;
    font-size: 16px;
    line-height: 1.5;
  }
  .brand {
    font: 600 14px var(--font-num);
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--accent);
  }
  h1 {
    font-size: 24px;
    font-weight: 600;
    margin: 0.5rem 0 1rem;
  }
  h2 {
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-3);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 2rem 0 0.5rem;
  }
  .go {
    display: inline-block;
    margin: 0.25rem 0;
    padding: 0.7rem 1.1rem;
    border-radius: 8px;
    background: var(--accent);
    color: var(--bg);
    font-weight: 600;
    text-decoration: none;
  }
  .steps {
    padding-left: 1.25rem;
  }
  .steps li {
    margin-bottom: 0.9rem;
  }
  .steps .muted {
    display: block;
    font-size: 14px;
  }
  ul {
    padding-left: 1.25rem;
  }
  .ok {
    color: var(--ok);
    font-weight: 600;
  }
  .bad {
    color: var(--bad);
  }
  .small {
    font-size: 13px;
  }
  code {
    font-family: var(--font-num);
  }
</style>
