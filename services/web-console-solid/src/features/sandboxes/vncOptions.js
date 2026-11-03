// The noVNC options of the desktop console, as pure functions.
//
// Proxmox's noVNC sidebar is a thin layer over one object: an `RFB` connection has
// properties for scaling, quality, compression and the cursor, and methods for
// keys and clipboard. Everything that can be decided without a connection lives
// here, so the canvas and the sidebar stay small and this file is testable with
// `node --test`.
import * as keySymModule from '@novnc/novnc/lib/input/keysym.js';

// The keysym table ships as CommonJS. Vite hands its `default` export through the
// namespace, Node wraps it once more; both shapes resolve to the plain table.
const KeySym = [
  keySymModule,
  keySymModule?.default,
  keySymModule?.default?.default,
].find((candidate) => candidate && candidate.XK_F1 !== undefined) || {};

/** One key that is pressed: the X11 keysym the server reads and the DOM code. */
const key = (keysym, code) => Object.freeze({ keysym, code });

const CTRL = key(KeySym.XK_Control_L, 'ControlLeft');
const ALT = key(KeySym.XK_Alt_L, 'AltLeft');
const WIN = key(KeySym.XK_Super_L, 'MetaLeft');
const SHIFT = key(KeySym.XK_Shift_L, 'ShiftLeft');

/** What a fresh desktop viewer starts with: Proxmox's defaults. */
export const DEFAULTS = Object.freeze({
  scaling: 'local',
  viewOnly: false,
  quality: 6,
  compression: 2,
  dotCursor: false,
  shared: true,
  reconnect: true,
  reconnectDelay: 5,
});

/** The three ways the picture meets the window. */
export const SCALINGS = Object.freeze(['local', 'remote', 'none']);

/** The key combos the sidebar sends, in the order they are pressed. */
export const KEYS = Object.freeze({
  ctrlAltDel: Object.freeze([CTRL, ALT, key(KeySym.XK_Delete, 'Delete')]),
  ...Object.fromEntries(
    Array.from({ length: 12 }, (_, index) => {
      const n = index + 1;
      return [`ctrlAltF${n}`, Object.freeze([CTRL, ALT, key(KeySym[`XK_F${n}`], `F${n}`)])];
    }),
  ),
  tab: Object.freeze([key(KeySym.XK_Tab, 'Tab')]),
  esc: Object.freeze([key(KeySym.XK_Escape, 'Escape')]),
  ctrl: Object.freeze([CTRL]),
  alt: Object.freeze([ALT]),
  win: Object.freeze([WIN]),
  shift: Object.freeze([SHIFT]),
  printScreen: Object.freeze([key(KeySym.XK_Print, 'PrintScreen')]),
});

/** The sticky modifiers of the Send key section, by the name in `KEYS`. */
export const STICKY = Object.freeze(['ctrl', 'alt', 'win', 'shift']);

// A number the sliders offer: an integer inside the range, or the fallback.
const whole = (value, low, high, fallback) =>
  typeof value === 'number' && Number.isFinite(value)
    ? Math.min(high, Math.max(low, Math.round(value)))
    : fallback;

/**
 * One complete, well-typed option object.
 *
 * A stored value from an older console (or a hand-edited localStorage) must never
 * make the viewer unusable, so every field falls back to its default.
 */
export function sanitizeOptions(raw) {
  const input = raw && typeof raw === 'object' ? raw : {};
  return {
    scaling: SCALINGS.includes(input.scaling) ? input.scaling : DEFAULTS.scaling,
    viewOnly: input.viewOnly === true,
    quality: whole(input.quality, 0, 9, DEFAULTS.quality),
    compression: whole(input.compression, 0, 9, DEFAULTS.compression),
    dotCursor: input.dotCursor === true,
    shared: input.shared === false ? false : DEFAULTS.shared,
    reconnect: input.reconnect === false ? false : DEFAULTS.reconnect,
    reconnectDelay: whole(input.reconnectDelay, 1, 60, DEFAULTS.reconnectDelay),
  };
}

/**
 * Put the options onto a live (or connecting) `RFB`.
 *
 * Clipping is set before scaling, because noVNC lets scaling win over clipping:
 * in the other order a switch from "None" to "Local scaling" would leave a
 * clipping viewport behind the scaled picture.
 *
 * @returns the options that were applied, so the caller can show them.
 */
export function applyOptions(rfb, options) {
  const wanted = sanitizeOptions({ ...DEFAULTS, ...(options || {}) });
  if (!rfb) return wanted;
  rfb.clipViewport = wanted.scaling === 'none';
  rfb.resizeSession = wanted.scaling === 'remote';
  rfb.scaleViewport = wanted.scaling === 'local';
  rfb.viewOnly = wanted.viewOnly;
  rfb.qualityLevel = wanted.quality;
  rfb.compressionLevel = wanted.compression;
  rfb.showDotCursor = wanted.dotCursor;
  return wanted;
}

/**
 * Press every key in order and release them in reverse.
 *
 * That is what a keyboard does with a combo: modifiers go down first, come up
 * last, and the key in the middle is the one the desktop reacts to.
 *
 * @param keys a `KEYS` entry, a name from `KEYS`, or a list of key objects.
 * @returns whether anything was sent.
 */
export function sendCombo(rfb, keys) {
  const list = Array.isArray(keys) ? keys : KEYS[keys] || [];
  if (!rfb || list.length === 0) return false;
  for (const entry of list) rfb.sendKey(entry.keysym, entry.code, true);
  for (let index = list.length - 1; index >= 0; index -= 1) {
    rfb.sendKey(list[index].keysym, list[index].code, false);
  }
  return true;
}

/**
 * The keys a send-key click actually sends: the sticky modifiers first, then the
 * key itself, without a modifier that is already part of the combo.
 */
export function withSticky(sticky, keys) {
  const held = (sticky || []).filter((name) => KEYS[name]?.length === 1);
  const list = Array.isArray(keys) ? keys : KEYS[keys] || [];
  const modifiers = held.map((name) => KEYS[name][0]);
  return [...modifiers, ...list.filter((entry) => !modifiers.some((m) => m.keysym === entry.keysym))];
}

/**
 * The words of the status bar: what the viewer is doing right now.
 *
 * @param state `idle`, `connecting`, `open` or `closed`.
 * @param details `{ size: {width, height} | null, quality, reason }`.
 */
export function statusText(state, details = {}) {
  if (state === 'open') {
    const size = details.size;
    const quality = whole(details.quality, 0, 9, DEFAULTS.quality);
    return size ? `Connected (${size.width}×${size.height}, quality ${quality})` : `Connected (quality ${quality})`;
  }
  if (state === 'connecting') return 'Connecting…';
  if (state === 'closed') return details.reason ? `Disconnected — ${details.reason}` : 'Disconnected';
  return 'Not connected';
}
