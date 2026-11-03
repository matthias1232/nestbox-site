// Arc maths for the Gauge: a 270 degree arc from 135 to 405 degrees, in a square around (cx, cy).

const START = 135;
const SWEEP = 270;

// The fraction value/max clamped to 0..1; a missing or zero max reads as 0.
export function fraction(value, max) {
  const v = Number(value);
  const m = Number(max);
  if (!Number.isFinite(v) || !Number.isFinite(m) || m <= 0) return 0;
  return Math.min(1, Math.max(0, v / m));
}

// A point on the circle at `degrees` (0 = right, clockwise because SVG y points down).
function point(cx, cy, radius, degrees) {
  const rad = (degrees * Math.PI) / 180;
  return [cx + radius * Math.cos(rad), cy + radius * Math.sin(rad)];
}

const round = (n) => Math.round(n * 100) / 100;

// The SVG path of the arc that is `value/max` full, drawn around the centre (cx, cy).
// Returns '' for an empty arc so the caller can skip drawing it.
export function arcPath(value, max, radius, cx = radius, cy = radius) {
  const f = fraction(value, max);
  if (f === 0) return '';
  const sweep = SWEEP * f;
  const [x1, y1] = point(cx, cy, radius, START);
  const [x2, y2] = point(cx, cy, radius, START + sweep);
  const large = sweep > 180 ? 1 : 0;
  return `M ${round(x1)} ${round(y1)} A ${radius} ${radius} 0 ${large} 1 ${round(x2)} ${round(y2)}`;
}

// True when the gauge should draw in the "bad" colour (90 % or more).
export function isHot(value, max) {
  return fraction(value, max) >= 0.9;
}
