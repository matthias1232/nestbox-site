// The display layout of a desktop sandbox, as the console edits it: which monitors
// exist, how large each one is, where it sits, and whether they are separate
// screens or one stretched screen (U09, U10a, U10c).
//
// This module is the console's copy of the rules, and it is a copy on purpose:
// the sandbox's `nestbox-display` tool and the Rust API's `domain::display`
// refuse the same layouts with the same sentences, so a monitor tab, the
// arrangement and the server never disagree about what is allowed. Every
// function here is pure — no DOM, no fetch, no state — which is why the
// arrangement and the tab strip can both use it.
//
// The layout document is exactly the one on the wire:
//
//   { mode: 'separate' | 'stretched',
//     monitors: [{ id, name, width, height, x, y, primary }] }

/** The narrowest side a monitor may have, in pixels. */
export const MIN_SIDE = 320;

/** The widest side a monitor may have: 8K wide. */
export const MAX_SIDE = 7680;

/** How many monitors one layout may hold. */
export const MAX_MONITORS = 8;

/** The widest framebuffer the appliance will transmit. */
export const MAX_BOUNDING_WIDTH = 16384;

/** The tallest framebuffer the appliance will transmit. */
export const MAX_BOUNDING_HEIGHT = 8640;

/** The largest total area the monitors may cover: four 4K screens. */
export const MAX_AREA = 36_000_000;

/** Each monitor is its own screen. */
export const MODE_SEPARATE = 'separate';

/** One logical monitor over the whole bounding box. */
export const MODE_STRETCHED = 'stretched';

/** The resolutions a person picks from; the same list the sidebar and the add popover offer. */
export const PRESETS = Object.freeze([
  { label: 'HD 1280×720', width: 1280, height: 720 },
  { label: 'WXGA 1366×768', width: 1366, height: 768 },
  { label: '1440×900', width: 1440, height: 900 },
  { label: 'Full HD 1920×1080', width: 1920, height: 1080 },
  { label: 'WUXGA 1920×1200', width: 1920, height: 1200 },
  { label: 'QHD 2560×1440', width: 2560, height: 1440 },
  { label: 'UWQHD 3440×1440', width: 3440, height: 1440 },
  { label: '4K UHD 3840×2160', width: 3840, height: 2160 },
  { label: '5K 5120×2880', width: 5120, height: 2880 },
  { label: 'Tablet 820×1180', width: 820, height: 1180 },
  { label: 'Phone 390×844', width: 390, height: 844 },
  { label: 'Portrait FHD 1080×1920', width: 1080, height: 1920 },
]);

/** The layout every desktop sandbox starts with: one 1440×900 monitor. */
export function defaultLayout() {
  return {
    mode: MODE_SEPARATE,
    monitors: [{ id: 'm1', name: 'Monitor 1', width: 1440, height: 900, x: 0, y: 0, primary: true }],
  };
}

/** The smallest rectangle around every monitor: `{ left, top, width, height, right, bottom }`. */
export function bounds(monitors) {
  const list = Array.isArray(monitors) ? monitors : [];
  if (list.length === 0) return { left: 0, top: 0, width: 0, height: 0, right: 0, bottom: 0 };
  const left = Math.min(...list.map((m) => m.x));
  const top = Math.min(...list.map((m) => m.y));
  const right = Math.max(...list.map((m) => m.x + m.width));
  const bottom = Math.max(...list.map((m) => m.y + m.height));
  return { left, top, width: right - left, height: bottom - top, right, bottom };
}

/** True when the two rectangles share any pixel. */
export function overlaps(first, second) {
  return first.x < second.x + second.width
    && second.x < first.x + first.width
    && first.y < second.y + second.height
    && second.y < first.y + first.height;
}

/** True when a whole edge of one rectangle meets the other, like a tiling. */
export function touches(first, second) {
  const verticalOverlap = Math.min(first.y + first.height, second.y + second.height)
    > Math.max(first.y, second.y);
  const horizontalOverlap = Math.min(first.x + first.width, second.x + second.width)
    > Math.max(first.x, second.x);
  const beside = verticalOverlap
    && (first.x + first.width === second.x || second.x + second.width === first.x);
  const above = horizontalOverlap
    && (first.y + first.height === second.y || second.y + second.height === first.y);
  return beside || above;
}

/**
 * Every reason this layout cannot be applied, in the order the server finds them.
 *
 * The sentences are the ones `nestbox-display` and the Rust API print for the
 * same layout, so the console can show the refusal before it is even sent.
 */
export function validate(layout) {
  const errors = [];
  if (!layout || typeof layout !== 'object') return ['a layout needs 1 to 8 monitors, not 0'];
  if (layout.mode !== MODE_SEPARATE && layout.mode !== MODE_STRETCHED) {
    errors.push(`mode must be "${MODE_SEPARATE}" or "${MODE_STRETCHED}"`);
  }
  const monitors = Array.isArray(layout.monitors) ? layout.monitors : [];
  if (monitors.length === 0 || monitors.length > MAX_MONITORS) {
    errors.push(`a layout needs 1 to ${MAX_MONITORS} monitors, not ${monitors.length}`);
  }

  const seen = [];
  let primaries = 0;
  let sizesOk = true;
  monitors.forEach((monitor, index) => {
    const id = typeof monitor.id === 'string' ? monitor.id : '';
    let label;
    if (id.trim() === '') {
      errors.push(`monitor ${index + 1} needs an id`);
      label = `monitor ${index + 1}`;
    } else {
      if (seen.includes(id)) errors.push(`monitor id ${id} is used twice`);
      seen.push(id);
      label = id;
    }

    for (const [field, value] of [['width', monitor.width], ['height', monitor.height]]) {
      if (!Number.isInteger(value) || value < MIN_SIDE || value > MAX_SIDE) {
        errors.push(`${label}: ${field} must be a whole number from ${MIN_SIDE} to ${MAX_SIDE}`);
        sizesOk = false;
      }
    }
    for (const [field, value] of [['x', monitor.x], ['y', monitor.y]]) {
      if (!Number.isInteger(value) || value < 0) {
        errors.push(`${label}: ${field} must be 0 or more`);
        sizesOk = false;
      }
    }
    if (monitor.primary) primaries += 1;
  });

  if (primaries > 1) errors.push('only one monitor can be primary');

  // Geometry needs every size and position to be valid: a half-checked layout
  // would report overlaps between monitors that do not exist.
  if (sizesOk && errors.length === 0) errors.push(...geometryErrors(monitors));

  return errors;
}

/** Overlap, gaps, bounding box and area: what a display dialog refuses. */
function geometryErrors(monitors) {
  const errors = [];
  monitors.forEach((first, index) => {
    monitors.slice(index + 1).forEach((second) => {
      if (overlaps(first, second)) errors.push(`monitors ${first.id} and ${second.id} overlap`);
    });
  });
  if (errors.length > 0) return errors;

  if (monitors.length > 1) {
    for (const monitor of monitors) {
      const touchesAnother = monitors
        .filter((other) => other !== monitor)
        .some((other) => touches(monitor, other));
      if (!touchesAnother) errors.push(`monitor ${monitor.id} must touch another monitor at an edge`);
    }
  }

  const box = bounds(monitors);
  if (box.width > MAX_BOUNDING_WIDTH || box.height > MAX_BOUNDING_HEIGHT) {
    errors.push(`the layout is ${box.width} x ${box.height}, above the ${MAX_BOUNDING_WIDTH} x ${MAX_BOUNDING_HEIGHT} limit`);
  }

  const area = monitors.reduce((sum, monitor) => sum + monitor.width * monitor.height, 0);
  if (area > MAX_AREA) errors.push(`the monitors cover ${area} pixels, above the ${MAX_AREA} pixel limit`);
  return errors;
}

/** The next free monitor id, so a removed tab's id is never reused by accident. */
export function nextMonitorId(monitors) {
  let n = 1;
  const used = new Set((monitors || []).map((monitor) => monitor.id));
  while (used.has(`m${n}`)) n += 1;
  return `m${n}`;
}

/**
 * Where a dragged monitor lands: the nearest free slot beside another monitor.
 *
 * Like the arrangement in an OS display dialog, a monitor never floats free and
 * never covers another one: the four slots around every neighbour (its right,
 * left, bottom and top edge) are the only places a drop can land, and the
 * nearest one wins. Every slot touches the neighbour, so the drop is always a
 * layout the server accepts, and the arrangement shows the same rectangle while
 * the pointer moves.
 */
export function snap(monitors, movedId, x, y) {
  const requested = { x: Math.round(x), y: Math.round(y) };
  const moved = (monitors || []).find((monitor) => monitor.id === movedId);
  if (!moved) return requested;
  const others = monitors.filter((monitor) => monitor.id !== movedId);
  if (others.length === 0) return { x: 0, y: 0 };

  const slots = [];
  for (const other of others) {
    slots.push(
      { x: other.x + other.width, y: other.y },
      { x: other.x - moved.width, y: other.y },
      { x: other.x, y: other.y + other.height },
      { x: other.x, y: other.y - moved.height },
    );
  }

  const distance = (point) => (point.x - requested.x) ** 2 + (point.y - requested.y) ** 2;
  const usable = slots
    .filter((point) => point.x >= 0 && point.y >= 0)
    .map((point) => ({ ...point, at: { ...moved, x: point.x, y: point.y } }))
    .filter((point) => !others.some((other) => overlaps(point.at, other)))
    .sort((a, b) => distance(a) - distance(b));

  if (usable.length === 0) return { x: Math.max(0, requested.x), y: Math.max(0, requested.y) };
  return { x: usable[0].x, y: usable[0].y };
}

/** The layout with one monitor moved, snapped to its neighbours. */
export function moveMonitor(layout, id, x, y) {
  const at = snap(layout.monitors, id, x, y);
  return withMonitors(layout, layout.monitors.map((monitor) => (
    monitor.id === id ? { ...monitor, ...at } : monitor
  )));
}

/**
 * The layout with one more monitor, touching the right edge (default) or the
 * bottom edge of what is there now — the same rule as `nestbox-display add`.
 */
export function addMonitor(layout, spec = {}) {
  const width = Math.round(spec.width || 1920);
  const height = Math.round(spec.height || 1080);
  const box = bounds(layout.monitors);
  const below = spec.position === 'below';
  const id = nextMonitorId(layout.monitors);
  const monitor = {
    id,
    name: `Monitor ${layout.monitors.length + 1}`,
    width,
    height,
    x: below ? box.left : box.right,
    y: below ? box.bottom : box.top,
  };
  return withMonitors(layout, [...layout.monitors, monitor]);
}

/** The layout without one monitor; the last one cannot be removed. */
export function removeMonitor(layout, id) {
  if (layout.monitors.length <= 1) return layout;
  const monitors = layout.monitors.filter((monitor) => monitor.id !== id);
  if (!monitors.some((monitor) => monitor.primary)) monitors[0] = { ...monitors[0], primary: true };
  return withMonitors(layout, monitors);
}

/** The layout with one monitor resized; its position stays, so the preview shows the truth. */
export function resizeMonitor(layout, id, width, height) {
  const next = {
    width: Math.round(Number(width) || 0),
    height: Math.round(Number(height) || 0),
  };
  return withMonitors(layout, layout.monitors.map((monitor) => (
    monitor.id === id ? { ...monitor, ...next } : monitor
  )));
}

/** The layout with one monitor turned by a quarter: a portrait screen is a swapped one. */
export function rotateMonitor(layout, id) {
  return withMonitors(layout, layout.monitors.map((monitor) => (
    monitor.id === id ? { ...monitor, width: monitor.height, height: monitor.width } : monitor
  )));
}

/** The layout with one monitor's name changed. */
export function renameMonitor(layout, id, name) {
  const label = String(name || '').trim().slice(0, 40);
  if (!label) return layout;
  return withMonitors(layout, layout.monitors.map((monitor) => (
    monitor.id === id ? { ...monitor, name: label } : monitor
  )));
}

/** The layout with one monitor marked primary; at most one carries it. */
export function setPrimary(layout, id) {
  return withMonitors(layout, layout.monitors.map((monitor) => {
    const { primary, ...rest } = monitor;
    return monitor.id === id ? { ...rest, primary: true } : rest;
  }));
}

/** The layout in another mode. */
export function setMode(layout, mode) {
  return { ...layout, mode };
}

/** The live counters under the arrangement: size, count, area and the limits that are hit. */
export function totals(layout) {
  const monitors = Array.isArray(layout?.monitors) ? layout.monitors : [];
  const box = bounds(monitors);
  const area = monitors.reduce((sum, monitor) => sum + (monitor.width || 0) * (monitor.height || 0), 0);
  return {
    width: box.width,
    height: box.height,
    count: monitors.length,
    area,
    megapixels: area / 1_000_000,
    overArea: area > MAX_AREA,
    overBox: box.width > MAX_BOUNDING_WIDTH || box.height > MAX_BOUNDING_HEIGHT,
    overCount: monitors.length > MAX_MONITORS,
  };
}

/**
 * The arrangement's drawing: rectangles in SVG pixels for a given width.
 *
 * The aspect ratio is kept, so a monitor is a rectangle and not a square, and
 * the inverse (`/ scale`) is how a drag becomes layout pixels again.
 */
export function arrangeGeometry(monitors, width, options = {}) {
  const pad = options.pad ?? 10;
  const maxHeight = options.maxHeight ?? 148;
  const box = bounds(monitors);
  const usableWidth = Math.max(1, width - pad * 2);
  const usableHeight = Math.max(1, maxHeight - pad * 2);
  const scale = Math.min(
    usableWidth / Math.max(1, box.width),
    usableHeight / Math.max(1, box.height),
  );
  return {
    width,
    height: Math.round(box.height * scale + pad * 2),
    scale,
    pad,
    box,
    rects: monitors.map((monitor) => ({
      id: monitor.id,
      x: Math.round(pad + (monitor.x - box.left) * scale),
      y: Math.round(pad + (monitor.y - box.top) * scale),
      w: Math.max(8, Math.round(monitor.width * scale)),
      h: Math.max(8, Math.round(monitor.height * scale)),
    })),
  };
}

/** The status line after an apply: `Applied 1920×1080 + 2560×1440 · separate · 0.3 s`. */
export function describeApplied(layout, elapsedMs) {
  const monitors = (layout?.monitors || [])
    .map((monitor) => `${monitor.width}×${monitor.height}`)
    .join(' + ');
  const seconds = Math.max(0, Math.round(Number(elapsedMs) || 0) / 1000).toFixed(1);
  return `Applied ${monitors} · ${layout?.mode || MODE_SEPARATE} · ${seconds} s`;
}

/** The status word of a mode, for the two buttons of the mode switch. */
export function modeLabel(mode) {
  return mode === MODE_STRETCHED ? 'One stretched screen' : 'Separate screens';
}

/** `1920×1080` for one monitor. */
export function monitorSize(monitor) {
  return `${monitor.width}×${monitor.height}`;
}

/** A copy of the layout with another monitor list. */
function withMonitors(layout, monitors) {
  return { ...layout, monitors };
}
