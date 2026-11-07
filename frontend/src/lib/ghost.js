// The target rower: where each joint should be at each moment of a good
// stroke, drawn with the rower's own proportions so it lies over the live
// figure. Coordinates are the live figure's: thigh lengths from where the
// feet rest, x toward the feet, y up. Joints, as everywhere: ear, shoulder,
// elbow, wrist, hip, knee, ankle, heel, toe.

export const J = { EAR: 0, SHOULDER: 1, ELBOW: 2, WRIST: 3, HIP: 4, KNEE: 5, ANKLE: 6, HEEL: 7, TOE: 8 };

const NEEDED = [J.SHOULDER, J.ELBOW, J.WRIST, J.HIP, J.KNEE, J.ANKLE];
/** Knee angle at the finish: legs long, not locked. */
const FINISH_KNEE_DEG = 172;

const dist = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1]);

function median(values) {
  const v = values.filter(Number.isFinite).sort((a, b) => a - b);
  return v.length ? v[Math.floor(v.length / 2)] : null;
}

/** The rower's proportions from recent live figures; null until enough are seen. */
export function measure(frames) {
  const seen = frames.filter((f) => f && NEEDED.every((j) => f[j]));
  if (seen.length < 30) return null;
  const seg = (a, b) => median(seen.filter((f) => f[a] && f[b]).map((f) => dist(f[a], f[b])));
  const coord = (j, k) => median(seen.filter((f) => f[j]).map((f) => f[j][k]));
  const point = (j) => {
    const x = coord(j, 0);
    const y = coord(j, 1);
    return x == null || y == null ? null : [x, y];
  };
  return {
    thigh: seg(J.HIP, J.KNEE),
    shin: seg(J.KNEE, J.ANKLE),
    trunk: seg(J.HIP, J.SHOULDER),
    upper: seg(J.SHOULDER, J.ELBOW),
    fore: seg(J.ELBOW, J.WRIST),
    head: seg(J.SHOULDER, J.EAR) ?? 0.45,
    // The seat runs level, and so does the handle.
    seat: coord(J.HIP, 1),
    handle: coord(J.WRIST, 1),
    ankle: point(J.ANKLE) ?? [0, 0],
    heel: point(J.HEEL),
    toe: point(J.TOE),
  };
}

/** The middle joint of a chain from `a` to `c` (lengths `l1` then `l2`), the side `pick` prefers. */
function middle(a, c, l1, l2, pick) {
  const full = dist(a, c);
  if (full < 1e-6) return [a[0], a[1] + l1];
  const d = Math.min(full, l1 + l2 - 1e-6);
  const [ux, uy] = [(c[0] - a[0]) / full, (c[1] - a[1]) / full];
  const along = (l1 * l1 - l2 * l2 + d * d) / (2 * d);
  const h = Math.sqrt(Math.max(0, l1 * l1 - along * along));
  const [bx, by] = [a[0] + ux * along, a[1] + uy * along];
  return pick([bx - uy * h, by + ux * h], [bx + uy * h, by - ux * h]);
}

const ease = (t) => {
  const x = Math.min(1, Math.max(0, t));
  return x * x * (3 - 2 * x);
};
const span = (u, from, to) => ease((u - from) / (to - from));

/**
 * The good stroke's body at `phase` of the stroke (0 the catch, `drive` the
 * finish, 1 the next catch). Drive: legs, then the body swings back, then
 * the arms draw in. Recovery: arms away, body over, then a slow slide.
 * `lean` is the body angle at the catch and finish, degrees, + toward the feet.
 */
export function targetPose(b, phase, drive, lean) {
  let legs;
  let body;
  let arms;
  if (phase < drive) {
    const u = phase / drive;
    legs = span(u, 0, 0.6);
    body = span(u, 0.3, 0.8);
    arms = span(u, 0.6, 1);
  } else {
    const v = (phase - drive) / (1 - drive);
    arms = 1 - span(v, 0, 0.2);
    body = 1 - span(v, 0.1, 0.35);
    legs = 1 - span(v, 0.3, 1);
  }
  const ankle = b.ankle;
  const height = b.seat - ankle[1];
  // Catch: shins vertical, the knee over the ankle. Finish: legs long.
  const rise = b.shin - height;
  const catchX = ankle[0] - Math.sqrt(Math.max(0, b.thigh ** 2 - rise ** 2));
  const bent = (FINISH_KNEE_DEG * Math.PI) / 180;
  const leg = Math.sqrt(b.thigh ** 2 + b.shin ** 2 - 2 * b.thigh * b.shin * Math.cos(bent));
  const finishX = ankle[0] - Math.sqrt(Math.max(0, leg ** 2 - height ** 2));
  const hip = [catchX + (finishX - catchX) * legs, b.seat];
  const knee = middle(ankle, hip, b.shin, b.thigh, (p, q) => (p[1] > q[1] ? p : q));

  const angle = ((lean.catch + (lean.finish - lean.catch) * body) * Math.PI) / 180;
  const dir = [Math.sin(angle), Math.cos(angle)];
  const shoulder = [hip[0] + b.trunk * dir[0], hip[1] + b.trunk * dir[1]];
  const ear = [shoulder[0] + b.head * dir[0], shoulder[1] + b.head * dir[1]];

  // The handle runs level: arms long out front, then drawn in to the ribs.
  const arm = (b.upper + b.fore) * 0.999;
  const drop = shoulder[1] - b.handle;
  const longX = shoulder[0] + Math.sqrt(Math.max(0, arm ** 2 - drop ** 2));
  const ribsX = hip[0] + (b.handle - b.seat) * Math.tan(angle) + 0.15;
  const wrist = [longX + (ribsX - longX) * arms, b.handle];
  const elbow = middle(shoulder, wrist, b.upper, b.fore, (p, q) => (p[1] < q[1] ? p : q));
  return [ear, shoulder, elbow, wrist, hip, knee, ankle, b.heel, b.toe];
}

/** Drive and recovery seconds at `rate`: a third of the stroke on the drive (as the server's `Rhythm::at`). */
export function rhythmAt(rate) {
  const cycle = 60 / rate;
  const drive_s = Math.min(1.1, Math.max(0.8, cycle / 3));
  return { rate, drive_s, recovery_s: cycle - drive_s };
}

/**
 * Catches and finishes from the live wrist path: the handle turns round at
 * each. A turn counts once the wrist is `turn` thigh lengths back from its
 * furthest point, and is dated to that point.
 */
export class StrokeClock {
  constructor(turn = 0.25) {
    this.turn = turn;
    this.state = null;
    this.extreme = null;
    this.catchAt = null;
    this.finishAt = null;
  }

  push(t, wristX) {
    if (wristX == null) return;
    if (this.state === null) {
      this.state = 'recovery';
      this.extreme = [t, wristX];
    } else if (this.state === 'recovery') {
      // Coming forward: the catch is the furthest forward point.
      if (wristX >= this.extreme[1]) this.extreme = [t, wristX];
      else if (this.extreme[1] - wristX > this.turn) {
        this.catchAt = this.extreme[0];
        this.state = 'drive';
        this.extreme = [t, wristX];
      }
    } else if (wristX <= this.extreme[1]) {
      this.extreme = [t, wristX];
    } else if (wristX - this.extreme[1] > this.turn) {
      this.finishAt = this.extreme[0];
      this.state = 'recovery';
      this.extreme = [t, wristX];
    }
  }
}
