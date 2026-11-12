// Where the rower is in the stroke by the clock, for the rhythm bar: when
// the last catch was (from the live figure's own handle, so it shares the
// figure's delay), and how long the drive and recovery should take.

/** The handle's joint in a live figure (ear, shoulder, elbow, wrist, …). */
export const WRIST = 3;

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
