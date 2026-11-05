// MediaPipe pose in the browser. Only landmarks leave the page, never video.
import { FilesetResolver, PoseLandmarker } from '@mediapipe/tasks-vision';

// MediaPipe landmark indices for model::POSE_JOINTS
// (ear, shoulder, elbow, wrist, hip, knee, ankle, heel, toe), left then right.
export const LEFT = [7, 11, 13, 15, 23, 25, 27, 29, 31];
export const RIGHT = [8, 12, 14, 16, 24, 26, 28, 30, 32];
// Bones between POSE_JOINTS positions, for drawing.
export const BONES = [
  [0, 1], // ear–shoulder
  [1, 2], // upper arm
  [2, 3], // forearm
  [1, 4], // trunk
  [4, 5], // thigh
  [5, 6], // shin
  [6, 7], // ankle–heel
  [7, 8], // foot
  [6, 8], // ankle–toe
];
export const VISIBLE = 0.5;

/** The landmarker, on the GPU when the browser allows it. */
export async function createLandmarker() {
  const fileset = await FilesetResolver.forVisionTasks('/mediapipe/wasm');
  const options = (delegate) => ({
    baseOptions: { modelAssetPath: '/mediapipe/pose_landmarker_full.task', delegate },
    runningMode: 'VIDEO',
    numPoses: 1,
  });
  try {
    return { landmarker: await PoseLandmarker.createFromOptions(fileset, options('GPU')), delegate: 'GPU' };
  } catch (e) {
    console.warn('Pose on GPU failed, using CPU:', e);
    return { landmarker: await PoseLandmarker.createFromOptions(fileset, options('CPU')), delegate: 'CPU' };
  }
}

const r4 = (v) => Math.round(v * 1e4) / 1e4;

/** `[x, y, visibility]` per joint, left side then right, or [] with no pose. */
export function pickJoints(landmarks) {
  if (!landmarks) return [];
  return [...LEFT, ...RIGHT].map((i) => {
    const l = landmarks[i];
    return [r4(l.x), r4(l.y), r4(l.visibility ?? 0)];
  });
}

/** One frame for /ws/camera: capture time in µs since the epoch. */
export function frameMessage(points, capturedUs, width, height) {
  return JSON.stringify({ t: Math.round(capturedUs), w: width, h: height, p: points });
}

/** Now on the page's clock, µs: the same base as frame capture times. */
export const pageNowUs = () => (performance.timeOrigin + performance.now()) * 1000;

/**
 * The page's clock against the server's (the page may be a phone with its
 * own clock). Each ping's reply carries the server time; assuming the reply
 * took half the round trip, offset = server − midpoint. The quickest round
 * trip of the last minute gives the best estimate, and re-pinging follows
 * any drift between the two clocks.
 */
export class ClockSync {
  #samples = [];

  ping() {
    return JSON.stringify({ ping: pageNowUs() });
  }

  /** A `{type: 'clock', ping, server_us}` reply. */
  reply(m) {
    const back = pageNowUs();
    this.#samples.push({ at: back, rtt: back - m.ping, offset: m.server_us - (m.ping + back) / 2 });
    this.#samples = this.#samples.filter((s) => back - s.at < 60e6);
  }

  get ready() {
    return this.#samples.length > 0;
  }

  /** Server minus page, µs, and the uncertainty (half the round trip). */
  get best() {
    return this.#samples.reduce((a, s) => (!a || s.rtt < a.rtt ? s : a), null);
  }

  /** Add to a page time to get server time. */
  get offset() {
    return this.best?.offset ?? 0;
  }
}

/** Which side the model sees better, for drawing before the server says. */
export function clearerSide(points) {
  if (points.length !== 18) return null;
  const sum = (from) => points.slice(from, from + 9).reduce((a, p) => a + p[2], 0);
  return sum(0) >= sum(9) ? 'left' : 'right';
}
