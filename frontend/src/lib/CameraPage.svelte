<script>
  // Camera setup and capture. Pose runs here in the browser; each frame's
  // landmarks go to the server (/ws/camera), which records them with the
  // session and answers with the framing check.
  import { onDestroy, onMount } from 'svelte';
  import { BONES, VISIBLE, ClockSync, clearerSide, createLandmarker, frameMessage, pickJoints } from './pose.js';

  // `phone`: the page runs on a phone (/phone/camera), full window.
  let { phone = false } = $props();

  const STORE_KEY = 'row-assist.camera';
  const remembered = () => {
    try {
      return localStorage.getItem(STORE_KEY) ?? '';
    } catch {
      return '';
    }
  };

  let video;
  let canvas;
  let devices = $state([]);
  let deviceId = $state(remembered());
  let stage = $state('Loading the pose model…');
  let problem = $state(null);
  let delegate = $state(null);
  let res = $state(null);
  let fps = $state(0);
  let link = $state('connecting');
  let recording = $state(false);
  let framing = $state(null);
  let clockOffset = $state(null);
  let awake = $state(null);

  let landmarker = null;
  let stream = null;
  let ws = null;
  let alive = true;
  let lastTs = 0;
  let frames = 0;
  let fpsFrom = performance.now();
  const clock = new ClockSync();
  let pingTimer;
  let wake = null;

  onMount(async () => {
    if (!window.isSecureContext || !navigator.mediaDevices?.getUserMedia) {
      problem =
        'Browsers only allow the camera on localhost or HTTPS. On the computer with the camera, open http://localhost:8080/camera; for a phone, start rowctl with --phone and open the address it prints.';
      return;
    }
    document.addEventListener('visibilitychange', onVisible);
    try {
      ({ landmarker, delegate } = await createLandmarker());
    } catch (e) {
      problem = `The pose model didn't load: ${e.message ?? e}. Was the frontend built (npm run build)?`;
      return;
    }
    connect();
    await openCamera();
  });

  onDestroy(() => {
    alive = false;
    clearTimeout(pingTimer);
    document.removeEventListener('visibilitychange', onVisible);
    wake?.release().catch(() => {});
    stream?.getTracks().forEach((t) => t.stop());
    ws?.close();
    landmarker?.close();
  });

  // A phone's screen locking stops the camera: hold it on while the page is
  // visible (the lock is dropped whenever the page is hidden).
  async function keepAwake() {
    if (wake && !wake.released) return;
    if (!navigator.wakeLock || document.visibilityState !== 'visible') {
      awake = false;
      return;
    }
    try {
      wake = await navigator.wakeLock.request('screen');
      awake = true;
      wake.addEventListener('release', () => (awake = false));
    } catch {
      awake = false;
    }
  }

  function onVisible() {
    if (document.visibilityState === 'visible' && stream) keepAwake();
  }

  async function openCamera() {
    stage = 'Opening the camera…';
    stream?.getTracks().forEach((t) => t.stop());
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        audio: false,
        video: {
          // A phone's back camera unless one was chosen; laptops ignore it.
          ...(deviceId ? { deviceId: { exact: deviceId } } : { facingMode: { ideal: 'environment' } }),
          width: { ideal: 1280 },
          height: { ideal: 720 },
          frameRate: { ideal: 30 },
        },
      });
    } catch (e) {
      if (deviceId && e.name === 'OverconstrainedError') {
        deviceId = '';
        return openCamera();
      }
      problem =
        e.name === 'NotAllowedError'
          ? 'Camera permission was refused. Allow it in the site settings and reload.'
          : e.name === 'NotFoundError'
            ? 'No camera found.'
            : `The camera didn't open: ${e.message}`;
      return;
    }
    const all = await navigator.mediaDevices.enumerateDevices();
    devices = all.filter((d) => d.kind === 'videoinput');
    const settings = stream.getVideoTracks()[0].getSettings();
    deviceId = settings.deviceId ?? deviceId;
    video.srcObject = stream;
    await video.play();
    res = `${video.videoWidth}×${video.videoHeight}`;
    stage = null;
    keepAwake();
    schedule();
  }

  function choose(id) {
    deviceId = id;
    try {
      localStorage.setItem(STORE_KEY, id);
    } catch {
      // Private mode: the choice just isn't remembered.
    }
    openCamera();
  }

  function connect() {
    if (!alive) return;
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    ws = new WebSocket(`${proto}://${location.host}/ws/camera`);
    link = 'connecting';
    // Five quick pings, then one every 5 s to follow drift.
    ws.onopen = () => {
      let n = 0;
      const tick = () => {
        if (ws?.readyState !== WebSocket.OPEN) return;
        ws.send(clock.ping());
        n += 1;
        pingTimer = setTimeout(tick, n < 5 ? 150 : 5000);
      };
      tick();
    };
    ws.onmessage = (e) => {
      const m = JSON.parse(e.data);
      if (m.type === 'clock') {
        clock.reply(m);
        clockOffset = clock.best;
      } else if (m.type === 'hello') {
        link = 'open';
        recording = m.recording;
      } else if (m.type === 'framing') {
        framing = m;
      } else if (m.type === 'error') {
        link = 'refused';
        problem = m.message;
      }
    };
    ws.onclose = () => {
      clearTimeout(pingTimer);
      if (link !== 'refused') link = 'closed';
      if (alive && link !== 'refused') setTimeout(connect, 2000);
    };
  }

  function schedule() {
    if (!alive) return;
    if ('requestVideoFrameCallback' in HTMLVideoElement.prototype) {
      video.requestVideoFrameCallback(onFrame);
    } else {
      requestAnimationFrame((now) => onFrame(now, null));
    }
  }

  function onFrame(now, meta) {
    if (!alive || !landmarker) return;
    // The model wants strictly increasing timestamps (ms).
    const ts = Math.max(performance.now(), lastTs + 1);
    lastTs = ts;
    // When the camera captured the frame, on the page's clock; the server
    // aligns it with the PM5 by wall time.
    const captured = meta?.captureTime ?? meta?.expectedDisplayTime ?? now;
    const capturedUs = (performance.timeOrigin + captured) * 1000;

    const result = landmarker.detectForVideo(video, ts);
    const points = pickJoints(result.landmarks?.[0]);
    draw(points);
    // Skip a frame rather than queue them if the socket is backed up, and
    // until the clocks are compared (frames go out on the server's clock).
    if (ws?.readyState === WebSocket.OPEN && clock.ready && ws.bufferedAmount < 64 * 1024) {
      ws.send(frameMessage(points, capturedUs + clock.offset, video.videoWidth, video.videoHeight));
    }
    frames++;
    if (ts - fpsFrom >= 1000) {
      fps = (frames * 1000) / (ts - fpsFrom);
      frames = 0;
      fpsFrom = ts;
    }
    schedule();
  }

  const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

  function draw(points) {
    const w = video.videoWidth;
    const h = video.videoHeight;
    if (canvas.width !== w) canvas.width = w;
    if (canvas.height !== h) canvas.height = h;
    const g = canvas.getContext('2d');
    g.clearRect(0, 0, w, h);
    if (points.length !== 18) return;
    const near = framing?.side ?? clearerSide(points);
    const scale = Math.max(1, w / 640);
    for (const side of ['far', 'near']) {
      const offset = (side === 'near') === (near === 'left') ? 0 : 9;
      const pts = points.slice(offset, offset + 9);
      const color = side === 'near' ? css('--accent') : css('--ink-3');
      g.strokeStyle = color;
      g.lineWidth = (side === 'near' ? 4 : 2) * scale;
      g.lineCap = 'round';
      for (const [a, b] of BONES) {
        if (pts[a][2] < VISIBLE || pts[b][2] < VISIBLE) continue;
        g.beginPath();
        g.moveTo(pts[a][0] * w, pts[a][1] * h);
        g.lineTo(pts[b][0] * w, pts[b][1] * h);
        g.stroke();
      }
      if (side === 'near') {
        for (const [x, y, v] of pts) {
          if (v < VISIBLE) continue;
          g.beginPath();
          g.arc(x * w, y * h, 5 * scale, 0, Math.PI * 2);
          g.fillStyle = color;
          g.fill();
          g.lineWidth = 2 * scale;
          g.strokeStyle = css('--surface');
          g.stroke();
        }
      }
    }
  }

  const linkText = $derived(
    {
      connecting: 'Connecting to Row Assist…',
      open: recording ? 'Recording with the session' : 'Setup only: not recording',
      closed: 'Lost the connection to Row Assist; retrying',
      refused: 'Another camera page is connected',
    }[link],
  );
</script>

<div class="camera" class:phone>
  <div class="stage">
    <!-- svelte-ignore a11y_media_has_caption -->
    <video bind:this={video} muted playsinline></video>
    <canvas bind:this={canvas} aria-label="Detected body landmarks"></canvas>
    {#if problem}
      <p class="overlay problem">{problem}</p>
    {:else if stage}
      <p class="overlay">{stage}</p>
    {/if}
  </div>

  <aside>
    <div class="status">
      <span class="dot" class:on={link === 'open' && recording} class:warn={link !== 'open'}></span>
      {linkText}
    </div>

    {#if devices.length > 1}
      <label>
        <span class="muted">Camera</span>
        <select value={deviceId} onchange={(e) => choose(e.currentTarget.value)}>
          {#each devices as d (d.deviceId)}
            <option value={d.deviceId}>{d.label || 'Camera'}</option>
          {/each}
        </select>
      </label>
    {/if}

    <dl class="facts">
      <div><dt>video</dt><dd class="num tab">{res ?? '–'}</dd></div>
      <div><dt>pose</dt><dd class="num tab">{fps ? `${fps.toFixed(0)} fps` : '–'}</dd></div>
      <div><dt>runs on</dt><dd class="num">{delegate ?? '–'}</dd></div>
      <div title="This page's clock against Row Assist's, corrected in every frame (± half the round trip)">
        <dt>clock</dt>
        <dd class="num tab">{clockOffset ? `${clockOffset.offset < 0 ? '−' : '+'}${Math.abs(clockOffset.offset / 1000).toFixed(0)} ms ±${(clockOffset.rtt / 2000).toFixed(0)}` : '–'}</dd>
      </div>
      <div><dt>screen</dt><dd class="num">{awake == null ? '–' : awake ? 'kept on' : 'may lock'}</dd></div>
    </dl>

    <h2>Setup check</h2>
    {#if framing}
      <ul class="checks">
        {#each framing.checks as c (c.key)}
          <li class:bad={!c.ok}>
            <span class="mark" aria-hidden="true">{c.ok ? '✓' : '✕'}</span>
            <span class="sr">{c.ok ? 'OK' : 'Problem'}:</span>
            {c.detail}
          </li>
        {/each}
      </ul>
      {#if framing.ok}
        <p class="ready">Ready to row.</p>
      {/if}
    {:else}
      <p class="muted">Waiting for frames…</p>
    {/if}

    <h2>Placement</h2>
    <ul class="tips muted">
      {#if phone}
        <li>Landscape and plugged in. Pick the Back Camera above, not Ultra Wide (it bends the edges).</li>
        {#if awake === false}
          <li>This browser can't keep the screen on: set Settings → Display &amp; Brightness → Auto-Lock to Never.</li>
        {/if}
        <li>The setup check also shows on the mirror, so you can check it from the erg.</li>
      {/if}
      <li>Square to the side of the erg, lens level with your hip, about 2–3 m away.</li>
      <li>Whole body in view at both the catch and the finish.</li>
      <li>Light on you from the front or side, not a bright window behind you.</li>
      <li>Fitted clothes help the model find your joints.</li>
    </ul>
  </aside>
</div>

<style>
  .camera.phone {
    padding: 1rem;
  }
  .camera {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 20rem;
    gap: 2rem;
    align-items: start;
  }
  .stage {
    position: relative;
    background: var(--bg);
    border: 1px solid var(--rule);
    border-radius: 4px;
    min-height: 12rem;
  }
  video,
  canvas {
    display: block;
    width: 100%;
    height: auto;
    max-height: 78vh;
    object-fit: contain;
  }
  canvas {
    position: absolute;
    inset: 0;
    height: 100%;
  }
  .overlay {
    position: absolute;
    inset: 0;
    margin: 0;
    display: grid;
    place-items: center;
    padding: 2rem;
    text-align: center;
    color: var(--ink-2);
  }
  .problem {
    color: var(--bad);
  }
  .status {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 14px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--ink-3);
  }
  .dot.on {
    background: var(--accent);
  }
  .dot.warn {
    background: var(--z1);
  }
  label {
    display: grid;
    gap: 0.25rem;
    margin-top: 1rem;
    font-size: 13px;
  }
  select {
    font: inherit;
    color: var(--ink);
    background: var(--bg);
    border: 1px solid var(--rule);
    border-radius: 4px;
    padding: 0.3rem;
  }
  .facts {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.25rem 1rem;
    margin: 1rem 0 0;
    font-size: 14px;
  }
  .facts div {
    display: contents;
  }
  .facts dt {
    color: var(--ink-3);
  }
  .facts dd {
    margin: 0;
    text-align: right;
  }
  h2 {
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-3);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 1.75rem 0 0.6rem;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.45rem;
    font-size: 14px;
  }
  .checks li {
    display: grid;
    grid-template-columns: 1.2rem 1fr;
    color: var(--ink-2);
  }
  .checks .mark {
    color: var(--accent);
  }
  .checks li.bad {
    color: var(--ink);
  }
  .checks li.bad .mark {
    color: var(--bad);
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
  .ready {
    margin: 0.75rem 0 0;
    color: var(--accent);
    font-weight: 600;
  }
  .tips li::before {
    content: '· ';
  }
  @media (max-width: 900px) {
    .camera {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
