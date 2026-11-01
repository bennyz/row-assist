export const DASH = '–';

const missing = (v) => v == null || Number.isNaN(v);

export function fixed(v, digits = 0) {
  return missing(v) ? DASH : v.toFixed(digits);
}

export function grouped(v) {
  return missing(v) ? DASH : Math.round(v).toLocaleString('en-US');
}

const pad = (n) => String(n).padStart(2, '0');

/** `m:ss`, or `h:mm:ss` past an hour. */
export function clock(seconds) {
  if (missing(seconds)) return DASH;
  const s = Math.max(0, Math.round(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h ? `${h}:${pad(m)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
}

/** Concept2 pace per 500 m from watts: `watts = 2.80 / (s/m)^3`. */
export function pace(watts) {
  if (missing(watts) || watts <= 0) return DASH;
  const tenths = Math.round(500 * Math.cbrt(2.8 / watts) * 10);
  return `${Math.floor(tenths / 600)}:${pad(Math.floor(tenths / 10) % 60)}.${tenths % 10}`;
}

export function dateLabel(us) {
  return new Date(us / 1000).toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' });
}

export function timeLabel(us) {
  return new Date(us / 1000).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hour12: false });
}

/** Round axis bounds and ~`count` clean steps covering [lo, hi]. */
export function niceTicks(lo, hi, count = 4) {
  if (!(hi > lo)) hi = lo + 1;
  const raw = (hi - lo) / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const norm = raw / mag;
  const step = (norm < 1.5 ? 1 : norm < 3 ? 2 : norm < 7 ? 5 : 10) * mag;
  const start = Math.floor(lo / step) * step;
  const end = Math.ceil(hi / step) * step;
  const ticks = [];
  for (let v = start; v <= end + step / 2; v += step) ticks.push(+v.toFixed(10));
  return { lo: start, hi: end, ticks };
}

/** SVG path through points, breaking the line where y is missing. */
export function linePath(points) {
  let d = '';
  let pen = false;
  for (const [x, y] of points) {
    if (missing(y)) {
      pen = false;
      continue;
    }
    d += `${pen ? 'L' : 'M'}${x.toFixed(1)},${y.toFixed(1)}`;
    pen = true;
  }
  return d;
}

/** Index of the item whose `key` is closest to `v` (items sorted by key). */
export function nearest(items, key, v) {
  let lo = 0;
  let hi = items.length - 1;
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if (items[mid][key] < v) lo = mid;
    else hi = mid;
  }
  return Math.abs(items[hi]?.[key] - v) < Math.abs(items[lo]?.[key] - v) ? hi : lo;
}
