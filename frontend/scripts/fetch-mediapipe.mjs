// Puts the pose model and its WASM runtime in public/mediapipe/, so the camera
// page works offline (at the erg). The runtime comes from node_modules; the
// model is downloaded once (~9 MB) and kept.
import { copyFile, mkdir, stat, writeFile, rename } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const out = join(root, 'public', 'mediapipe');
const MODEL = 'pose_landmarker_full.task';
const MODEL_URL = `https://storage.googleapis.com/mediapipe-models/pose_landmarker/pose_landmarker_full/float16/1/${MODEL}`;
// forVisionTasks() picks SIMD or not at runtime; the ES-module variant isn't used.
const WASM = ['vision_wasm_internal.js', 'vision_wasm_internal.wasm', 'vision_wasm_nosimd_internal.js', 'vision_wasm_nosimd_internal.wasm'];

const exists = (p) => stat(p).then(() => true, () => false);

await mkdir(join(out, 'wasm'), { recursive: true });
for (const f of WASM) {
  await copyFile(join(root, 'node_modules/@mediapipe/tasks-vision/wasm', f), join(out, 'wasm', f));
}

const model = join(out, MODEL);
if (!(await exists(model))) {
  console.log(`Downloading ${MODEL}…`);
  const res = await fetch(MODEL_URL);
  if (!res.ok) throw new Error(`${MODEL_URL}: ${res.status} ${res.statusText}`);
  await writeFile(`${model}.part`, Buffer.from(await res.arrayBuffer()));
  await rename(`${model}.part`, model);
}
console.log(`MediaPipe pose model and runtime in ${out}`);
