// A single monitor of the desktop, cropped out of the one framebuffer noVNC
// transmits (U10b).
//
// A desktop sandbox has one framebuffer: the bounding box of its monitor
// layout. VNC sends that whole picture. "Show only monitor 2" is therefore a
// viewport question, not a second stream: noVNC's display can show a rectangle
// of the framebuffer at a scale, and its pointer maths (`absX = x / scale +
// viewport.x`) turns a click in the canvas back into framebuffer pixels.
//
// This module is the only place that touches `rfb._display`. It is a private
// object, so it is written down here once, with the pure maths beside it that
// can be tested without a browser — and with the fallback in `DesktopTab` for a
// noVNC that no longer has it.
//
// The maths, once, so the proof has something to compare against:
//
//   scale = min(box.w / monitor.width, box.h / monitor.height)
//   canvas size (CSS px) = monitor.width * scale × monitor.height * scale
//   a click at (cx, cy) in the canvas is framebuffer (cx / scale + monitor.x,
//                                                          cy / scale + monitor.y)

/** How many CSS pixels one framebuffer pixel takes when the monitor fills the box. */
export function fitScale(monitor, box) {
  const width = Number(box?.w) || 0;
  const height = Number(box?.h) || 0;
  if (!monitor || !(monitor.width > 0) || !(monitor.height > 0)) return 1;
  if (!(width > 0) || !(height > 0)) return 1;
  return Math.min(width / monitor.width, height / monitor.height);
}

/** The canvas size a cropped monitor view needs, in CSS pixels. */
export function cropSize(monitor, box) {
  const scale = fitScale(monitor, box);
  return {
    width: Math.max(1, Math.round(monitor.width * scale)),
    height: Math.max(1, Math.round(monitor.height * scale)),
    scale,
  };
}

/**
 * The framebuffer pixel a click on the cropped canvas lands on.
 *
 * `point` is in CSS pixels from the canvas's top-left corner, which is what
 * noVNC's own pointer handler reads; the answer is absolute in the framebuffer,
 * the coordinate the sandbox's X server uses.
 */
export function clickToFramebuffer(monitor, box, point) {
  const { scale } = cropSize(monitor, box);
  return {
    x: Math.round(point.x / scale + monitor.x),
    y: Math.round(point.y / scale + monitor.y),
  };
}

/** The viewport a cropped monitor view needs: the monitor's rectangle, in framebuffer pixels. */
export function viewportFor(monitor) {
  return { x: monitor.x, y: monitor.y, w: monitor.width, h: monitor.height };
}

/** True when this noVNC build has the display object the crop needs. */
export function canCrop(rfb) {
  const display = rfb?._display;
  return Boolean(display
    && typeof display.viewportChangeSize === 'function'
    && typeof display.viewportChangePos === 'function'
    && display._viewportLoc);
}

/**
 * Show one monitor, scaled to the box, with correct clicks.
 *
 * Clipping is set before scaling, for the reason `vncOptions.applyOptions`
 * documents: noVNC turns clipping off when scaling is on, so the other order
 * leaves a picture of the whole framebuffer. Returns false when this noVNC has
 * no display object, so the caller can fall back instead of guessing.
 */
export function showMonitor(rfb, monitor, box) {
  if (!monitor || !canCrop(rfb)) return false;
  const display = rfb._display;
  rfb.scaleViewport = false;
  rfb.clipViewport = true;
  display.viewportChangeSize(monitor.width, monitor.height);
  const at = display._viewportLoc;
  display.viewportChangePos(monitor.x - at.x, monitor.y - at.y);
  display.scale = fitScale(monitor, box);
  return true;
}

/** Show the whole framebuffer the way the viewer's scaling option asks for. */
export function showAll(rfb) {
  if (!rfb) return false;
  rfb.clipViewport = false;
  rfb.scaleViewport = true;
  return true;
}
