// Tunnel wording: how a node's WireGuard state reads in the table.
//
// The server answers `up`, `stale`, `down` or `unknown` plus the age of the last
// handshake. The age is the part an operator reads first ("did it just drop?"),
// so it is appended to the state word instead of getting its own column. Plain
// JS, so the wording is testable without a DOM.

const TUNNEL_TONE = {
  up: 'ok',
  stale: 'warn',
  down: 'bad',
  unknown: 'muted',
};

/** The badge colour for a tunnel state; anything unclassified stays neutral. */
export function tunnelTone(state) {
  return TUNNEL_TONE[String(state || '').toLowerCase()] || 'muted';
}

/**
 * Seconds as a short duration: `12 s`, `4 min`, `1 h 05 min`, `2 d`.
 *
 * An absent age is an empty string, so a caller can leave it out instead of
 * printing "null s". A tunnel's age is always small, but the same text is used
 * for a stale peer, which can be minutes old.
 */
export function ageText(seconds) {
  if (seconds === null || seconds === undefined) return '';
  const total = Math.floor(Number(seconds));
  if (!Number.isFinite(total)) return '';
  const safe = Math.max(0, total);
  if (safe < 60) return `${safe} s`;
  if (safe < 3600) return `${Math.floor(safe / 60)} min`;
  if (safe < 86400) {
    const hours = Math.floor(safe / 3600);
    const minutes = Math.floor((safe % 3600) / 60);
    return minutes === 0 ? `${hours} h` : `${hours} h ${String(minutes).padStart(2, '0')} min`;
  }
  return `${Math.floor(safe / 86400)} d`;
}

/**
 * The state word with its age, e.g. `up · 12 s`.
 *
 * A missing state reads as `unknown`: a node nobody has a tunnel opinion about
 * is not the same as one that is up, and the table must not imply otherwise.
 */
export function tunnelLabel(state, age) {
  const word = String(state || 'unknown').toLowerCase();
  const ageRead = ageText(age);
  return ageRead ? `${word} · ${ageRead}` : word;
}
