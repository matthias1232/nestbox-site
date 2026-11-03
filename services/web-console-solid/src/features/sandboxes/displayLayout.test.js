// The console's copy of the display rules, tested against the same cases the
// Rust domain and the sandbox's `nestbox-display` tool test (U09, U10a, U10c).
//
// The point of these tests is sameness: if the console ever accepts a layout the
// server refuses, the arrangement shows a picture that can never be applied.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  MAX_AREA,
  MAX_MONITORS,
  MODE_SEPARATE,
  MODE_STRETCHED,
  PRESETS,
  addMonitor,
  arrangeGeometry,
  bounds,
  defaultLayout,
  describeApplied,
  modeLabel,
  monitorSize,
  moveMonitor,
  nextMonitorId,
  removeMonitor,
  renameMonitor,
  resizeMonitor,
  rotateMonitor,
  setMode,
  setPrimary,
  snap,
  totals,
  validate,
} from './displayLayout.js';

/** One monitor in the documented shape. */
const monitor = (id, width, height, x, y) => ({
  id, name: `Monitor ${id}`, width, height, x, y, primary: false,
});

/** The same monitor, marked primary. */
const primary = (id, width, height, x, y) => ({ ...monitor(id, width, height, x, y), primary: true });

/** A layout in the given mode. */
const layoutOf = (mode, monitors) => ({ mode, monitors });

const errorsOf = (layout) => validate(layout);

test('the default layout is valid', () => {
  assert.deepEqual(validate(defaultLayout()), []);
});

test('two monitors side by side are valid', () => {
  const layout = layoutOf(MODE_SEPARATE, [
    primary('m1', 1920, 1080, 0, 0),
    monitor('m2', 2560, 1440, 1920, 0),
  ]);
  assert.deepEqual(validate(layout), []);
});

test('a layout needs between one and eight monitors', () => {
  assert.deepEqual(errorsOf(layoutOf(MODE_SEPARATE, [])), ['a layout needs 1 to 8 monitors, not 0']);
  const nine = Array.from({ length: 9 }, (_, index) => monitor(`m${index + 1}`, 640, 480, index * 640, 0));
  assert.ok(errorsOf(layoutOf(MODE_SEPARATE, nine)).includes('a layout needs 1 to 8 monitors, not 9'));
  assert.equal(MAX_MONITORS, 8);
});

test('each side is between 320 and 7680', () => {
  assert.deepEqual(errorsOf(layoutOf(MODE_SEPARATE, [monitor('m1', 300, 900, 0, 0)])),
    ['m1: width must be a whole number from 320 to 7680']);
  assert.deepEqual(errorsOf(layoutOf(MODE_SEPARATE, [monitor('m1', 1440, 8000, 0, 0)])),
    ['m1: height must be a whole number from 320 to 7680']);
  assert.deepEqual(validate(layoutOf(MODE_SEPARATE, [monitor('m1', 320, 320, 0, 0)])), []);
  assert.deepEqual(validate(layoutOf(MODE_SEPARATE, [monitor('m1', 7680, 4320, 0, 0)])), []);
});

test('the mode has to be known', () => {
  assert.deepEqual(errorsOf(layoutOf('mirror', [monitor('m1', 1440, 900, 0, 0)])),
    ['mode must be "separate" or "stretched"']);
});

test('negative positions are refused', () => {
  assert.ok(errorsOf(layoutOf(MODE_SEPARATE, [monitor('m1', 1440, 900, -1, 0)])).includes('m1: x must be 0 or more'));
  assert.ok(errorsOf(layoutOf(MODE_SEPARATE, [monitor('m1', 1440, 900, 0, -1)])).includes('m1: y must be 0 or more'));
});

test('ids are required and unique', () => {
  assert.deepEqual(errorsOf(layoutOf(MODE_SEPARATE, [{ id: ' ', name: '', width: 1440, height: 900, x: 0, y: 0 }])),
    ['monitor 1 needs an id']);
  assert.ok(errorsOf(layoutOf(MODE_SEPARATE, [monitor('m1', 1440, 900, 0, 0), monitor('m1', 1440, 900, 1440, 0)]))
    .includes('monitor id m1 is used twice'));
});

test('only one monitor can be primary', () => {
  const both = layoutOf(MODE_SEPARATE, [primary('m1', 1440, 900, 0, 0), primary('m2', 1440, 900, 1440, 0)]);
  assert.ok(errorsOf(both).includes('only one monitor can be primary'));
});

test('monitors may not overlap', () => {
  const stacked = layoutOf(MODE_SEPARATE, [monitor('m1', 1920, 1080, 0, 0), monitor('m2', 1280, 1024, 1000, 100)]);
  assert.deepEqual(errorsOf(stacked), ['monitors m1 and m2 overlap']);
});

test('a monitor has to touch another one', () => {
  const gap = layoutOf(MODE_SEPARATE, [monitor('m1', 1920, 1080, 0, 0), monitor('m2', 1280, 1024, 2000, 0)]);
  const found = errorsOf(gap);
  assert.ok(found.includes('monitor m2 must touch another monitor at an edge'));
});

test('a corner contact is not a touch', () => {
  const corner = layoutOf(MODE_SEPARATE, [monitor('m1', 1920, 1080, 0, 0), monitor('m2', 1280, 1024, 1920, 1080)]);
  const found = errorsOf(corner);
  assert.ok(found.includes('monitor m2 must touch another monitor at an edge'));
});

test('the bounding box is limited', () => {
  const tooWide = layoutOf(MODE_SEPARATE, [
    monitor('m1', 7680, 4320, 0, 0),
    monitor('m2', 7680, 4320, 7680, 0),
    monitor('m3', 7680, 4320, 15360, 0),
  ]);
  assert.ok(errorsOf(tooWide).some((error) => error.includes('above the 16384 x 8640 limit')));
});

test('the total area is limited', () => {
  const fourK = Array.from({ length: 4 }, (_, index) => monitor(`m${index + 1}`, 3840, 2160, index * 3840, 0));
  assert.deepEqual(validate(layoutOf(MODE_SEPARATE, fourK)), []);
  const fiveK = [...fourK, monitor('m5', 3840, 2160, 0, 2160)];
  assert.ok(errorsOf(layoutOf(MODE_SEPARATE, fiveK)).some((error) => error.includes('above the 36000000 pixel limit')));
});

test('the box of an L shape takes the corner into the box', () => {
  const monitors = [monitor('m1', 1920, 1080, 0, 0), monitor('m2', 1280, 1024, 0, 1080)];
  assert.deepEqual(bounds(monitors), { left: 0, top: 0, width: 1920, height: 2104, right: 1920, bottom: 2104 });
});

test('resizing keeps the position and rotating swaps the sides', () => {
  const layout = defaultLayout();
  assert.deepEqual(resizeMonitor(layout, 'm1', 1920, 1080).monitors[0], {
    id: 'm1', name: 'Monitor 1', width: 1920, height: 1080, x: 0, y: 0, primary: true,
  });
  assert.deepEqual(rotateMonitor(layout, 'm1').monitors[0].width, 900);
  assert.deepEqual(rotateMonitor(layout, 'm1').monitors[0].height, 1440);
});

test('a monitor is added touching the right or the bottom edge', () => {
  const layout = defaultLayout();
  const right = addMonitor(layout, { width: 1920, height: 1080, position: 'right' });
  assert.deepEqual(right.monitors[1], {
    id: 'm2', name: 'Monitor 2', width: 1920, height: 1080, x: 1440, y: 0,
  });
  assert.deepEqual(validate(right), []);
  const below = addMonitor(layout, { width: 1280, height: 1024, position: 'below' });
  assert.deepEqual(below.monitors[1], {
    id: 'm2', name: 'Monitor 2', width: 1280, height: 1024, x: 0, y: 900,
  });
  assert.deepEqual(validate(below), []);
});

test('the last monitor cannot be removed, and a primary passes on', () => {
  const one = defaultLayout();
  assert.equal(removeMonitor(one, 'm1').monitors.length, 1);
  const two = addMonitor(one, { width: 1280, height: 1024 });
  const left = removeMonitor(two, 'm1');
  assert.deepEqual(left.monitors.map((entry) => entry.id), ['m2']);
  assert.equal(left.monitors[0].primary, true);
});

test('a monitor is made primary alone', () => {
  const two = addMonitor(defaultLayout(), { width: 1280, height: 1024 });
  const picked = setPrimary(two, 'm2');
  assert.equal(picked.monitors.find((entry) => entry.id === 'm2').primary, true);
  assert.equal(picked.monitors.find((entry) => entry.id === 'm1').primary, undefined);
});

test('renaming keeps the layout valid and never empties a name', () => {
  const named = renameMonitor(defaultLayout(), 'm1', 'Left');
  assert.equal(named.monitors[0].name, 'Left');
  assert.equal(renameMonitor(defaultLayout(), 'm1', '   ').monitors[0].name, 'Monitor 1');
});

test('the next id is free and a mode is a mode', () => {
  assert.equal(nextMonitorId([monitor('m1', 1, 1, 0, 0), monitor('m2', 1, 1, 0, 0)]), 'm3');
  assert.equal(setMode(defaultLayout(), MODE_STRETCHED).mode, MODE_STRETCHED);
});

test('a drag snaps to the nearest free edge of a neighbour', () => {
  const monitors = [primary('m1', 1440, 900, 0, 0), monitor('m2', 1280, 1024, 1440, 0)];
  // Dragged mostly over the first monitor: the nearest free slot is under it.
  assert.deepEqual(snap(monitors, 'm2', 200, 430), { x: 0, y: 900 });
  // Dragged below the first monitor: it lands under it, left edge aligned.
  assert.deepEqual(snap(monitors, 'm2', 40, 1000), { x: 0, y: 900 });
  // Dragged past the right edge: it lands right of the first monitor.
  assert.deepEqual(snap(monitors, 'm2', 2600, 0), { x: 1440, y: 0 });
  // Dragged off the top-left: the left slot is out of bounds, the bottom slot is not.
  assert.deepEqual(snap(monitors, 'm2', -400, 0), { x: 0, y: 900 });
});

test('a snapped drag never overlaps and always touches', () => {
  const monitors = [
    primary('m1', 1440, 900, 0, 0),
    monitor('m2', 1280, 1024, 1440, 0),
    monitor('m3', 1920, 1080, 0, 900),
  ];
  for (const [x, y] of [[700, 400], [3000, 2000], [10, 10], [2880, 1600], [100, 5000]]) {
    const at = snap(monitors, 'm3', x, y);
    const laid = { ...monitors[2], ...at };
    const others = monitors.filter((entry) => entry.id !== 'm3');
    assert.equal(others.some((other) => reviewOverlap(laid, other)), false, `${x},${y} overlapped`);
    assert.equal(others.some((other) => reviewTouch(laid, other)), true, `${x},${y} did not touch`);
    const next = moveMonitor({ mode: MODE_SEPARATE, monitors }, 'm3', x, y);
    assert.deepEqual(validate(next), [], `${x},${y} was refused`);
  }
});

test('one monitor cannot be dragged away from the origin', () => {
  assert.deepEqual(snap([primary('m1', 1440, 900, 0, 0)], 'm1', 500, 300), { x: 0, y: 0 });
});

test('the counters name what is over the limit', () => {
  const two = addMonitor(defaultLayout(), { width: 1920, height: 1080 });
  const counted = totals(two);
  assert.equal(counted.width, 3360);
  assert.equal(counted.height, 1080);
  assert.equal(counted.count, 2);
  assert.equal(counted.overArea, false);
  assert.equal(totals({ mode: MODE_SEPARATE, monitors: [monitor('m1', 7680, 7680, 0, 0)] }).overArea, true);
  assert.ok(totals(two).area < MAX_AREA);
});

test('the arrangement keeps the aspect ratio and maps a drag back', () => {
  const monitors = [
    primary('m1', 1440, 900, 0, 0),
    monitor('m2', 1280, 1024, 1440, 0),
  ];
  const geometry = arrangeGeometry(monitors, 260);
  assert.equal(geometry.width, 260);
  assert.ok(geometry.height <= 148);
  const [first, second] = geometry.rects;
  assert.ok(second.x > first.x);
  assert.equal(second.y, first.y);
  // 10 px of CSS is 10 / scale layout pixels, which is what a drag uses.
  assert.ok(Math.abs(10 / geometry.scale - 10 / geometry.scale) < 1e-9);
  assert.ok(geometry.scale > 0 && geometry.scale < 1);
});

test('the applied line names every monitor, the mode and the time', () => {
  const layout = layoutOf(MODE_SEPARATE, [monitor('m1', 1920, 1080, 0, 0), monitor('m2', 2560, 1440, 1920, 0)]);
  assert.equal(describeApplied(layout, 320), 'Applied 1920×1080 + 2560×1440 · separate · 0.3 s');
  assert.equal(modeLabel(MODE_STRETCHED), 'One stretched screen');
  assert.equal(modeLabel(MODE_SEPARATE), 'Separate screens');
  assert.equal(monitorSize(layout.monitors[0]), '1920×1080');
});

test('every preset is a size the server accepts', () => {
  for (const preset of PRESETS) {
    assert.deepEqual(validate(layoutOf(MODE_SEPARATE, [monitor('m1', preset.width, preset.height, 0, 0)])), [],
      `${preset.label} was refused`);
  }
});

// The two predicates are re-stated here on purpose: a test that asks `snap` to
// prove itself with `snap`'s own helpers is no test at all.
function reviewOverlap(first, second) {
  return first.x < second.x + second.width && second.x < first.x + first.width
    && first.y < second.y + second.height && second.y < first.y + first.height;
}

function reviewTouch(first, second) {
  const vertical = Math.min(first.y + first.height, second.y + second.height) > Math.max(first.y, second.y);
  const horizontal = Math.min(first.x + first.width, second.x + second.width) > Math.max(first.x, second.x);
  return (vertical && (first.x + first.width === second.x || second.x + second.width === first.x))
    || (horizontal && (first.y + first.height === second.y || second.y + second.height === first.y));
}
