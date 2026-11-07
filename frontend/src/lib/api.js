async function getJson(url) {
  const res = await fetch(url);
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.error ?? `${res.status} ${res.statusText}`);
  }
  return res.json();
}

export const listSessions = () => getJson('/api/sessions');
export const getSession = (id) => getJson(`/api/sessions/${id}`);
export const getHealth = () => getJson('/api/health');
export const getProgress = () => getJson('/api/progress');
export const getToday = () => getJson('/api/today');
export const getPhone = () => getJson('/api/phone');
export const getRow = () => getJson('/api/row');
export const stopRow = () => fetch('/api/row/stop', { method: 'POST' });

/** What's rowed on purpose from now on: a fault key, or null for clean. */
export const markRow = (fault) =>
  fetch('/api/row/mark', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ fault }),
  });

/** How a row felt: effort 1–10 (or null) and a comment. */
export const setFeel = (id, effort, comment) =>
  fetch(`/api/sessions/${id}/feel`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ effort, comment }),
  });
