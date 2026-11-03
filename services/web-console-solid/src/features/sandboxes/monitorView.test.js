// The cropped per-monitor view, tested without a browser: the maths is pure, and
// the noVNC display object is faked so the call order is checked too.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  canCrop,
  clickToFramebuffer,
  cropSize,
  fitScale,
  showAll,
  showMonitor,
  viewportFor,
} from './monitorView.js';

const monitor = (id, x, y, width, height) => ({ id, name: id, x, y, width, height });

// A display object shaped like noVNC 1.5.0's, recording what was asked of it.
function fakeRfb() {
  const calls = [];
  const rfb = {
    scaleViewport: true,
    clipViewport: false,
    calls,
    _display: {
      _viewportLoc: { x: 0, y: 0, w: 1440, h: 900 },
      _scale: 1,
      get scale() { return this._scale; },
      set scale(value) { calls.push(['scale', value]); this._scale = value; },
      viewportChangeSize(w, h) { calls.push(['size', w, h]); this._viewportLoc.w = w; this._viewportLoc.h = h; },
      viewportChangePos(dx, dy) {
        calls.push(['pos', dx, dy]);
        this._viewportLoc.x += dx;
        this._viewportLoc.y += dy;
      },
      // noVNC 1.5.0's own pointer maths, so the claim is about the viewport the
      // code left behind, not about a copy of the formula.
      absX(x) { return x / this._scale + this._viewportLoc.x; },
      absY(y) { return y / this._scale + this._viewportLoc.y; },
    },
  };
  return rfb;
}

test('the scale is the smaller of the two ratios', () => {
  const second = monitor('m2', 1920, 0, 1280, 1024);
  assert.equal(fitScale(second, { w: 640, h: 1024 }), 0.5);
  assert.equal(fitScale(second, { w: 1280, h: 100 }), 100 / 1024);
  assert.equal(fitScale(second, { w: 1280, h: 1024 }), 1);
  // A box nobody measured is no reason for a zero scale.
  assert.equal(fitScale(second, { w: 0, h: 0 }), 1);
  assert.equal(fitScale({ width: 0, height: 0 }, { w: 100, h: 100 }), 1);
});

test('the cropped canvas is the monitor at its scale, never zero', () => {
  const second = monitor('m2', 1920, 0, 1280, 1024);
  assert.deepEqual(cropSize(second, { w: 640, h: 512 }), { width: 640, height: 512, scale: 0.5 });
  assert.deepEqual(cropSize(second, { w: 0, h: 0 }), { width: 1280, height: 1024, scale: 1 });
});

test('a click on the canvas maps back into framebuffer pixels', () => {
  // The two-monitor layout of the U10b proof (U10b step 1).
  const second = monitor('m2', 1920, 0, 1280, 1024);
  const box = { w: 640, h: 512 };
  // The centre of the canvas: half of monitor 2, plus its left origin.
  assert.deepEqual(clickToFramebuffer(second, box, { x: 320, y: 256 }), { x: 1920 + 640, y: 512 });
  // The top-left corner of the canvas is the monitor's own origin.
  assert.deepEqual(clickToFramebuffer(second, box, { x: 0, y: 0 }), { x: 1920, y: 0 });
  // A 1920×1080 monitor filling a 1920×1080 box is one pixel for one pixel.
  const first = monitor('m1', 0, 0, 1920, 1080);
  assert.deepEqual(clickToFramebuffer(first, { w: 1920, h: 1080 }, { x: 960, y: 540 }), { x: 960, y: 540 });
});

test('the viewport is the monitor rectangle', () => {
  assert.deepEqual(viewportFor(monitor('m3', 3200, 1080, 1920, 1080)), { x: 3200, y: 1080, w: 1920, h: 1080 });
});

test('showing a monitor clips, sizes, moves and then scales', () => {
  const rfb = fakeRfb();
  const second = monitor('m2', 1920, 0, 1280, 1024);
  assert.equal(showMonitor(rfb, second, { w: 640, h: 512 }), true);
  assert.equal(rfb.scaleViewport, false);
  assert.equal(rfb.clipViewport, true);
  assert.deepEqual(rfb.calls, [['size', 1280, 1024], ['pos', 1920, 0], ['scale', 0.5]]);
  // The pointer maths noVNC uses on the result: canvas centre, scaled, moved.
  const display = rfb._display;
  assert.equal(display.absX(320), 1920 + 640);
  assert.equal(display.absY(256), 512);
});

test('showing another monitor moves the viewport by the difference, not to it', () => {
  const rfb = fakeRfb();
  showMonitor(rfb, monitor('m2', 1920, 0, 1280, 1024), { w: 1280, h: 1024 });
  showMonitor(rfb, monitor('m1', 0, 0, 1440, 900), { w: 1440, h: 900 });
  assert.deepEqual(rfb.calls.slice(-2), [['pos', -1920, 0], ['scale', 1]]);
  assert.deepEqual(rfb._display._viewportLoc, { x: 0, y: 0, w: 1440, h: 900 });
});

test('an old noVNC is refused instead of guessed at', () => {
  assert.equal(canCrop({}), false);
  assert.equal(canCrop({ _display: {} }), false);
  assert.equal(showMonitor({ _display: {} }, monitor('m1', 0, 0, 800, 600), { w: 800, h: 600 }), false);
  assert.equal(showMonitor(fakeRfb(), null, { w: 800, h: 600 }), false);
});

test('showing everything turns clipping off before scaling on', () => {
  const rfb = fakeRfb();
  rfb.clipViewport = true;
  assert.equal(showAll(rfb), true);
  assert.equal(rfb.clipViewport, false);
  assert.equal(rfb.scaleViewport, true);
});
