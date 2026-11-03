// Pure time formatting for the Countdown component, kept in a plain module so a
// node test can load it without JSX or a DOM.

// Milliseconds as "12:05" under an hour, "1 h 05 min" from an hour, "expired" at or below zero.
export function formatRemaining(ms) {
  const value = Number(ms);
  if (ms == null || Number.isNaN(value) || value <= 0) return 'expired';
  const totalSeconds = Math.floor(value / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) return `${hours} h ${String(minutes).padStart(2, '0')} min`;
  return `${minutes}:${String(seconds).padStart(2, '0')}`;
}

// Milliseconds left until an ISO time, from `now` (ms since epoch); null for no time.
export function remainingMs(untilIso, now) {
  if (untilIso == null) return null;
  const until = new Date(untilIso).getTime();
  if (Number.isNaN(until)) return null;
  return until - Number(now || 0);
}
