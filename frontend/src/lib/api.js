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
