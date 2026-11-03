// Turns a list of numbers into the points string of an SVG polyline.

// The `points` attribute for `values` in a w x h box; `max` defaults to the largest value.
// One value draws a flat line; an empty list returns ''.
export function polyline(values, w, h, max) {
  const list = (values || []).filter((v) => v !== null && v !== '' && v !== undefined).map(Number).filter((n) => Number.isFinite(n));
  if (list.length === 0) return '';
  const top = Number.isFinite(max) && max > 0 ? max : Math.max(...list, 0) || 1;
  const pad = 1;
  const step = list.length > 1 ? w / (list.length - 1) : 0;
  return list
    .map((v, i) => {
      const x = list.length > 1 ? i * step : w / 2;
      const y = h - pad - (Math.min(Math.max(v, 0), top) / top) * (h - 2 * pad);
      return `${Math.round(x * 100) / 100},${Math.round(y * 100) / 100}`;
    })
    .join(' ');
}
