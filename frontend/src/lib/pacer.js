// The pacer's stroke: how the good stroke moves through its drive and
// recovery in time (the pace ghost), and the steps the stroke bar names.

// The handle through the drive: off the catch at once (the legs), easing
// into the finish. Half even, half smoothstep: smoothstep alone sat at the
// catch, then raced through the middle.
export const driveEase = (x) => 0.5 * x + 0.5 * x * x * (3 - 2 * x);
// Through the recovery: hands away and body over quickly, then a slow slide.
export const recoveryEase = (x) => 1 - (1 - x) ** 1.6;

// Where each step of the good stroke begins, as a share of the way through
// its drive or its recovery (the good order `rowctl report` shows: the body
// swings from 40% of the drive and the arms bend from 72%; on the way back
// the body goes over from 29% and the knees bend from 63%).
const STEPS = [
  { name: 'Legs', drive: true, at: 0 },
  { name: 'Swing', drive: true, at: 0.4 },
  { name: 'Arms', drive: true, at: 0.72 },
  { name: 'Hands away', drive: false, at: 0 },
  { name: 'Body over', drive: false, at: 0.29 },
  { name: 'Slide', drive: false, at: 0.63 },
];

/** The share of its time `ease` takes to get `s` of the way. */
function until(ease, s) {
  if (s <= 0) return 0;
  let [lo, hi] = [0, 1];
  for (let i = 0; i < 24; i++) {
    const mid = (lo + hi) / 2;
    if (ease(mid) < s) lo = mid;
    else hi = mid;
  }
  return (lo + hi) / 2;
}

/** Each step's start and length as shares of one stroke at `rhythm`,
 *  timed as the pace ghost rows it. */
export function stepsAt(rhythm) {
  const cycle = rhythm.drive_s + rhythm.recovery_s;
  const from = STEPS.map((s) =>
    s.drive
      ? (until(driveEase, s.at) * rhythm.drive_s) / cycle
      : (rhythm.drive_s + until(recoveryEase, s.at) * rhythm.recovery_s) / cycle,
  );
  return STEPS.map((s, i) => ({ ...s, from: from[i], width: (from[i + 1] ?? 1) - from[i] }));
}
