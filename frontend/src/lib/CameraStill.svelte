<script>
  // What the side camera sees, for setting it up from the erg: a still from
  // the phone every couple of seconds while open. It's held in memory on
  // this computer until the next one and never saved.
  import { onDestroy } from 'svelte';

  const EVERY_MS = 2000;

  let open = $state(false);
  let n = $state(0);
  let failed = $state(false);
  let timer;

  const next = (ms) => {
    clearTimeout(timer);
    timer = setTimeout(() => n++, ms);
  };
  function toggle() {
    open = !open;
    failed = false;
    clearTimeout(timer);
    if (open) n++;
  }
  onDestroy(() => clearTimeout(timer));
</script>

<section aria-label="Camera view">
  <button onclick={toggle}>{open ? 'Hide the camera view' : 'Show what the camera sees'}</button>
  {#if open}
    <img
      src={`/api/camera/still.jpg?n=${n}`}
      alt="What the side camera sees, with the detected body drawn on"
      class:stale={failed}
      onload={() => {
        failed = false;
        next(EVERY_MS);
      }}
      onerror={() => {
        failed = true;
        next(EVERY_MS * 2);
      }}
    />
    {#if failed}
      <p class="muted">No picture from the phone: is the camera page open and in front?</p>
    {/if}
  {/if}
</section>

<style>
  section {
    margin-bottom: 1.5rem;
  }
  button {
    font-size: 16px;
    padding: 0.4rem 0.9rem;
  }
  img {
    display: block;
    margin-top: 0.75rem;
    max-width: 100%;
    max-height: 45vh;
    border-radius: 6px;
    background: var(--rule);
  }
  img.stale {
    opacity: 0.4;
  }
  .muted {
    margin: 0.5rem 0 0;
    font-size: 16px;
    color: var(--ink-3);
  }
</style>
