// The task workspace's look-away rule: a face out of frame, or a head tilted
// down further than the learner's own calibration allows, for longer than the
// set's threshold. Pure functions over detector samples, so the rule is tested
// without a camera.
//
// It reads the face box and the detector's keypoints and nothing else: no eye
// gaze, no expression. What it estimates is how far the head is pitched down,
// which is what reading a phone in the lap looks like and reading the screen
// does not. It is a deterrent with known blind spots (a phone held at screen
// height, an overlay under the webcam), stated in docs/task-mode.md.
import { FACE_SAMPLE_GAP_MS } from "./face-presence.js";

/// How far down the head is pitched, larger meaning further down, or null
/// when the sample carries no geometry. With keypoints it is where the nose
/// sits between the eyes and the mouth, which moves toward the mouth as the
/// head tilts forward; without them it falls back to where the face box sits
/// in the frame. `kind` says which, so a calibration is only ever compared
/// with samples measured the same way.
export function downMetric(face) {
  if (!face?.box) return null;
  const points = face.landmarks ?? [];
  if (points.length >= 4) {
    const eyes = (points[0][1] + points[1][1]) / 2;
    const span = points[3][1] - eyes;
    if (span > 1e-3)
      return { kind: "keypoints", value: (points[2][1] - eyes) / span };
  }
  return { kind: "box", value: face.box[1] };
}

function quantile(values, q) {
  const sorted = [...values].sort((a, b) => a - b);
  if (!sorted.length) return null;
  const at = Math.min(
    sorted.length - 1,
    Math.max(0, Math.round(q * (sorted.length - 1))),
  );
  return sorted[at];
}

/// The smallest margin, in the metric's own units, between the head held for
/// reading and the head counted as looking down; a steadier learner gets this
/// rather than a limit inside the detector's noise.
const MIN_MARGIN = 0.08;
/// How far past the deepest normal keyboard glance the limit sits.
const KEYBOARD_MARGIN = 0.05;
/// The share of calibration samples that must show exactly one face.
const MIN_FACE_SHARE = 0.7;
/// Samples each calibration phase needs before it can be judged.
export const MIN_PHASE_SAMPLES = 10;

/// Sets the learner's look-away limit from two phases: looking at the screen,
/// then typing. The limit sits beyond both the reading posture and the usual
/// keyboard dip, so glancing at the keys is never "looking down"; a short
/// glance would not count anyway, being shorter than the threshold.
export function calibrate(screenSamples, typingSamples) {
  const measured = (samples) =>
    samples
      .filter((sample) => sample?.available && sample.count === 1)
      .map((sample) => downMetric(sample.face))
      .filter(Boolean);
  if (
    screenSamples.length < MIN_PHASE_SAMPLES ||
    typingSamples.length < MIN_PHASE_SAMPLES
  )
    return { ok: false, reason: "too_few_samples" };
  const screen = measured(screenSamples);
  if (screen.length < MIN_FACE_SHARE * screenSamples.length)
    return { ok: false, reason: "face_not_steady" };
  const kind = screen[0].kind;
  const values = screen
    .filter((row) => row.kind === kind)
    .map((row) => row.value);
  const baseline = quantile(values, 0.5);
  const spread = quantile(
    values.map((value) => Math.abs(value - baseline)),
    0.9,
  );
  const typing = measured(typingSamples)
    .filter((row) => row.kind === kind)
    .map((row) => row.value);
  // An unmeasured typing phase would stand the reading posture in for the
  // keyboard one, and every glance down while typing would then count.
  if (typing.length < MIN_FACE_SHARE * typingSamples.length)
    return { ok: false, reason: "face_not_steady" };
  const keyboard = quantile(typing, 0.9);
  const limit = Math.max(
    baseline + Math.max(4 * spread, MIN_MARGIN),
    keyboard + KEYBOARD_MARGIN,
  );
  const round = (value) => Math.round(value * 1000) / 1000;
  return {
    ok: true,
    metric: kind,
    baseline: round(baseline),
    keyboard: round(keyboard),
    limit: round(limit),
    screenSamples: screenSamples.length,
    typingSamples: typingSamples.length,
  };
}

/// Turns timed samples into the rule's state. `state` is `ok` while the face
/// is in frame and up, `away` once it is not, `warning` from half the
/// threshold, `violation` at the threshold, and `interrupted` when no
/// successful sample arrived for `gapMs`.
export function createLookAwayMonitor({
  limit,
  metric,
  thresholdMs,
  // A sampling gap this long means nothing was watched, a technical
  // interruption rather than a look-away: the interview's face tracker bound.
  gapMs = FACE_SAMPLE_GAP_MS,
}) {
  let lastGood = null;
  let lastSample = null;
  let lastMissed = false;
  let awaySince = null;
  const away = (now) => {
    const awayMs = now - awaySince;
    if (awayMs >= thresholdMs) return { state: "violation", awayMs };
    if (awayMs >= thresholdMs / 2) return { state: "warning", awayMs };
    return { state: "away", awayMs };
  };
  return {
    update(sample, now) {
      // Watching starts at the first sample, successful or not: a detector
      // that never answers after Start is unwatched time too.
      lastGood ??= now;
      const sinceSample = now - (lastSample ?? now);
      lastSample = now;
      // The interval after a missed sample was not observed either, so a
      // look-away resumed after a gap counts only from this sample.
      const afterMiss = lastMissed;
      lastMissed = !sample?.available;
      if (!sample?.available) {
        if (now - lastGood >= gapMs) return { state: "interrupted", awayMs: 0 };
        if (awaySince === null) return { state: "ok", awayMs: 0 };
        // Nothing was seen since the last sample, so that time is not
        // counted as looking away: the look-away clock pauses.
        awaySince += sinceSample;
        return away(now);
      }
      if (now - lastGood >= gapMs) {
        lastGood = now;
        return { state: "interrupted", awayMs: 0 };
      }
      lastGood = now;
      const measured = sample.count >= 1 ? downMetric(sample.face) : null;
      const down =
        measured && measured.kind === metric && measured.value > limit;
      if (sample.count >= 1 && !down) {
        awaySince = null;
        return { state: "ok", awayMs: 0 };
      }
      if (awaySince !== null && afterMiss) awaySince += sinceSample;
      awaySince ??= now;
      return away(now);
    },
  };
}
