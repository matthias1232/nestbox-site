// The pure parts of the live stream: handler registry and reconnect delay.

/** First and last reconnect delay in milliseconds. */
export const BACKOFF_START_MS = 1000;
export const BACKOFF_MAX_MS = 30000;

/** The delay after `previous` (0 or undefined means the first failure). */
export function nextDelay(previous) {
  if (!previous) return BACKOFF_START_MS;
  return Math.min(BACKOFF_MAX_MS, previous * 2);
}

/** A name to handlers registry. `on` returns a function that removes the handler. */
export function createRegistry() {
  const handlers = new Map();
  return {
    on(name, handler) {
      if (!handlers.has(name)) handlers.set(name, new Set());
      handlers.get(name).add(handler);
      return () => handlers.get(name)?.delete(handler);
    },
    emit(name, payload) {
      for (const handler of [...(handlers.get(name) || [])]) {
        try {
          handler(payload);
        } catch {
          // One broken listener must not cost the others their event.
        }
      }
    },
    count(name) {
      return handlers.get(name)?.size || 0;
    },
  };
}

/** The payload a handler receives: an alert event carries the alert view of contract C9. */
export function payloadFor(name, data) {
  if (name === 'alert' && data && typeof data === 'object' && data.alert) return data.alert;
  return data;
}
