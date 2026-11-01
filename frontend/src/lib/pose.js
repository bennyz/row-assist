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

/** Which side the model sees better, for drawing before the server says. */
export function clearerSide(points) {
  if (points.length !== 18) return null;
  const sum = (from) => points.slice(from, from + 9).reduce((a, p) => a + p[2], 0);
  return sum(0) >= sum(9) ? 'left' : 'right';
}
