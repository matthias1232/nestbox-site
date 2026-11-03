// Keyboard shortcut parsing and matching as pure functions.
//
// A combo is one step (`mod+k`, `?`, `Escape`) or a sequence of steps separated by spaces
// (`g h`). `mod` is Ctrl on Linux and Windows and Cmd on macOS.

/** The longest pause between the two keys of a sequence. */
export const SEQUENCE_WINDOW_MS = 800;

/** Parse `mod+k` or `g h` into steps. */
export function parseCombo(combo) {
  return String(combo)
    .trim()
    .split(/\s+/)
    .map((part) => {
      const pieces = part.split('+');
      const key = pieces.pop();
      const flags = new Set(pieces.map((piece) => piece.toLowerCase()));
      return {
        key: key.length === 1 ? key.toLowerCase() : key,
        mod: flags.has('mod'),
        shift: flags.has('shift'),
        alt: flags.has('alt'),
      };
    });
}

/** Whether one keyboard event is one parsed step. */
export function stepMatches(step, event, isMac = false) {
  const key = event.key && event.key.length === 1 ? event.key.toLowerCase() : event.key;
  if (key !== step.key) return false;
  const modDown = isMac ? Boolean(event.metaKey) : Boolean(event.ctrlKey);
  const otherMod = isMac ? Boolean(event.ctrlKey) : Boolean(event.metaKey);
  if (step.mod !== modDown || otherMod) return false;
  if (Boolean(event.altKey) !== step.alt) return false;
  // Shift matters for letters and named keys; a symbol such as `?` already implies it.
  const isSymbol = step.key.length === 1 && !/[a-z0-9]/.test(step.key);
  if (!isSymbol && Boolean(event.shiftKey) !== step.shift) return false;
  return true;
}

/** Whether the event came from somewhere the user is typing. */
export function isTypingTarget(target) {
  if (!target) return false;
  const tag = String(target.tagName || '').toLowerCase();
  if (tag === 'input' || tag === 'textarea' || tag === 'select') return true;
  if (target.isContentEditable) return true;
  // The terminal (xterm) keeps its focus in a hidden textarea inside this class.
  return typeof target.closest === 'function' && Boolean(target.closest('.xterm'));
}

/**
 * A matcher for one combo. `feed(event, now)` returns true when the combo just completed.
 * A wrong key in the middle of a sequence starts over.
 */
export function createMatcher(combo, { isMac = false, windowMs = SEQUENCE_WINDOW_MS } = {}) {
  const steps = parseCombo(combo);
  let index = 0;
  let last = 0;
  return {
    feed(event, now = Date.now()) {
      if (index > 0 && now - last > windowMs) index = 0;
      if (stepMatches(steps[index], event, isMac)) {
        index += 1;
        last = now;
        if (index === steps.length) {
          index = 0;
          return true;
        }
        return false;
      }
      index = stepMatches(steps[0], event, isMac) && steps.length > 1 ? 1 : 0;
      last = now;
      return false;
    },
  };
}
