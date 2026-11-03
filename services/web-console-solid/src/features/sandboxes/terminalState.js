// What the terminal's status line says, and how long to wait before trying again.
//
// A WebSocket has more than two states and an operator needs the difference: "still
// opening", "open", and "closed and given up" look the same on a dead cursor, and
// only the last one is a reason to press Reconnect. The wording and the backoff are
// pure functions here, so `node --test` proves them without a browser.
//
// The list is exactly Proxmox's habit: try again by itself, waiting longer each
// time, and stop after ten tries rather than hammering a sandbox that is asleep.
// `Reconnect` in the toolbar is the manual way back and starts the count over.

/** How many times the console reopens the socket on its own before it stops. */
export const MAX_TRIES = 10;

/** The waits between automatic tries, in milliseconds; the last one repeats. */
export const RETRY_DELAYS = [1000, 2000, 5000, 10000];

/**
 * The wait before automatic try number `attempt` (0-based).
 *
 * 1 s, 2 s, 5 s, then 10 s for every further try: a dropped connection to a local
 * container is usually back within a second, while a stopped sandbox should not be
 * asked sixty times a minute. A negative or missing attempt means the first one.
 */
export function nextDelay(attempt) {
  const index = Math.min(Math.max(Math.trunc(Number(attempt)) || 0, 0), RETRY_DELAYS.length - 1);
  return RETRY_DELAYS[index];
}

/** Whether automatic try number `attempt` (0-based) may still happen. */
export function shouldRetry(attempt) {
  return (Math.trunc(Number(attempt)) || 0) < MAX_TRIES;
}

/**
 * The status line's sentence, without the coloured dot.
 *
 * `state` is the socket's: `connecting`, `open`, or `closed`. `reason` is what the
 * last failure said — the server's close reason, the HTTP status behind a refused
 * handshake, or the retry countdown — and is appended rather than replaced, because
 * "Disconnected" alone tells nobody what to do next.
 */
export function statusText(state, reason) {
  const detail = typeof reason === 'string' ? reason.trim() : '';
  if (state === 'open') return 'Connected';
  if (state === 'connecting') return detail ? `Connecting… ${detail}` : 'Connecting…';
  return `Disconnected — ${detail || 'the connection closed'}`;
}

/** Which colour the dot takes: lime when open, amber while trying, red when not. */
export function statusTone(state) {
  if (state === 'open') return 'ok';
  if (state === 'connecting') return 'warn';
  return 'error';
}

/** The server's own reason for ending the session, in words a person reads. */
const CLOSE_REASONS = {
  idle: 'the shell was idle, so the server detached it',
  user: 'the session was closed',
  sandbox_stopped: 'the sandbox stopped',
  error: 'the session ended with an error',
};

/**
 * The sentence for a `{"type":"close"}` frame the server sent, or null.
 *
 * The frame carries a stable `reason` and an optional `message`. Both are shown:
 * the reason names the rule, the message is the runner's own detail.
 */
export function closeReasonText(frame) {
  const reason = typeof frame?.reason === 'string' ? frame.reason.trim() : '';
  const message = typeof frame?.message === 'string' ? frame.message.trim() : '';
  const label = CLOSE_REASONS[reason] || (reason ? `the server closed the session (${reason})` : '');
  if (label && message) return `${label}: ${message}`;
  return label || message || null;
}

/**
 * What a `CloseEvent` says, for the cases the server did not explain.
 *
 * A refused handshake reaches JavaScript as code 1006 with no text — the browser
 * hides the HTTP status on purpose — so this returns null there and the caller asks
 * the API for the real answer instead of inventing one.
 */
export function socketReasonText(event, opened) {
  const reason = typeof event?.reason === 'string' ? event.reason.trim() : '';
  if (reason) return reason;
  const code = Number(event?.code);
  if (code === 1000) return opened ? 'the server closed the connection' : null;
  if (code === 1005) return opened ? 'the connection closed' : null;
  if (code === 1006) return null;
  return Number.isFinite(code) && code > 0 ? `the connection closed (code ${code})` : null;
}

/**
 * The status line between two automatic tries, e.g.
 * `the server answered 404: … · next try in 2 s (try 2 of 10)`.
 */
export function retryText(why, attempt) {
  const wait = nextDelay(attempt) / 1000;
  const next = `next try in ${wait} s (try ${Math.trunc(Number(attempt)) + 1} of ${MAX_TRIES})`;
  const detail = typeof why === 'string' ? why.trim() : '';
  return detail ? `${detail} · ${next}` : next;
}
