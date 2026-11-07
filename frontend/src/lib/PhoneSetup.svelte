<script>
  // Shown on the laptop while no camera is connected: the code the phone
  // scans to reach its camera page. The address can change (Wi-Fi, the
  // router), so it's asked for again every so often.
  import { onDestroy } from 'svelte';
  import { getPhone } from './api.js';

  const RECHECK_MS = 15000;

  let phone = $state(null);
  let checked = $state(false);
  let timer;

  async function check() {
    try {
      phone = await getPhone();
    } catch {
      phone = null;
    }
    checked = true;
    timer = setTimeout(check, RECHECK_MS);
  }
  check();
  onDestroy(() => clearTimeout(timer));
</script>

{#if phone}
  <div class="phone">
    <img src={`/api/phone/qr.svg?u=${encodeURIComponent(phone.setup_url)}`} width="220" height="220" alt="A code that opens {phone.setup_url}" />
    <div>
      <h2>Camera</h2>
      <p>On the iPhone, open the camera page from its bookmark, or scan this code.</p>
      <p class="url">{phone.setup_url}</p>
      <p class="muted">
        The first time, the page walks you through trusting Row Assist once. Then bookmark the camera page so
        it’s one tap next time.
      </p>
    </div>
  </div>
{:else if checked}
  <p class="muted">Waiting for a network address for the phone camera. Is Wi-Fi on?</p>
{/if}

<style>
  .phone {
    display: flex;
    gap: 2rem;
    align-items: center;
  }
  img {
    flex: none;
    border-radius: 6px;
    background: #fff;
  }
  h2 {
    font-size: 15px;
    font-weight: 500;
    color: var(--ink-3);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 0 0 0.6rem;
  }
  p {
    margin: 0 0 0.6rem;
    font-size: 20px;
    color: var(--ink);
    max-width: 32rem;
  }
  .url {
    font: 16px var(--font-num);
    color: var(--ink-2);
  }
  .muted {
    font-size: 16px;
    color: var(--ink-3);
  }
  @media (max-width: 600px) {
    .phone {
      flex-direction: column;
      align-items: start;
    }
  }
</style>
