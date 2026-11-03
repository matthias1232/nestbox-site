// The alert pop-up mute, as pure functions (no DOM, no signals).
//
// `alertPopups` in uiPrefs is 'on', 'off', or { until: <ms epoch> }. 'off' means "until I turn
// them on", the object is a timed mute. An expired time counts as 'on', so a one-hour mute never
// outlives its hour because nobody touched the console. The counter is not part of this file:
// muting or hiding a pop-up leaves `open().length`, and with it the bell, untouched.

/**
 * Resolve a stored `alertPopups` value to 'on' or 'off' at `now` (ms epoch).
 *
 * A timed mute whose moment has passed resolves to 'on'; a malformed value does too, because an
 * unreadable preference must not hide refusals the operator never asked to hide.
 */
export function resolvePopups(value, now = Date.now()) {
  if (value === 'off') return 'off';
  if (value && typeof value === 'object' && Number.isFinite(value.until)) {
    return value.until > now ? 'off' : 'on';
  }
  return 'on';
}

/**
 * The moment a live timed mute ends (ms epoch), or null.
 *
 * The Alerts tab names that moment ("muted until 15:40"); an untimed mute and a mute that already
 * ran out answer null, so the tab says "muted" or "on" instead of a time in the past.
 */
export function popupsUntil(value, now = Date.now()) {
  if (value && typeof value === 'object' && Number.isFinite(value.until) && value.until > now) {
    return value.until;
  }
  return null;
}
