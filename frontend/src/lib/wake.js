// Keep the screen on while the page is visible: a phone's screen locking
// stops its camera, and the laptop in front of the erg would blank mid-row.
// The browser drops the lock whenever the page is hidden, so it is taken
// again each time the page comes back. `onchange(held)` hears whether it
// is held; the function returned lets it go.
export function keepScreenOn(onchange = () => {}) {
  let lock = null;
  let stopped = false;
  async function take() {
    if (stopped || (lock && !lock.released)) return;
    if (!navigator.wakeLock || document.visibilityState !== 'visible') {
      onchange(false);
      return;
    }
    try {
      lock = await navigator.wakeLock.request('screen');
    } catch {
      onchange(false);
      return;
    }
    // Let go while the request was out: the page is leaving.
    if (stopped) {
      lock.release().catch(() => {});
      return;
    }
    onchange(true);
    lock.addEventListener('release', () => onchange(false));
  }
  const onVisible = () => {
    if (document.visibilityState === 'visible') take();
  };
  document.addEventListener('visibilitychange', onVisible);
  take();
  return () => {
    stopped = true;
    document.removeEventListener('visibilitychange', onVisible);
    lock?.release().catch(() => {});
  };
}
