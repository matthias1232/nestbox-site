// The pure parts of the live stream: event names, handler registry, reference counts
// and the reconnect delay. Nothing here touches the browser, so it is unit-tested.

/** First and last reconnect delay in milliseconds. */
export const BACKOFF_START_MS = 1000;
export const BACKOFF_MAX_MS = 30000;

/**
 * The state stream's event names (contract C18).
 *
 * A window holds exactly one of these; a screen subscribes to a name instead of
 * opening a stream of its own.
 */
export const STATE_EVENTS = ['sandbox', 'gateway', 'job', 'policy_rollout', 'control', 'node',
  'chat_started', 'chat_delta', 'chat_done', 'models', 'alert'];

/** The telemetry stream's event names (contract C18). */
export const TELEMETRY_EVENTS = ['network', 'dns', 'inference'];

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

/**
 * A reference count for one shared stream.
 *
 * `acquire()` hands back the function that gives that one request back, so a
 * screen that mounts twice releases exactly once, and a double release is a
 * no-op. `watch(fn)` is told only when the count crosses zero: a second screen
 * asking for a stream that is already open must not restart it, which is the
 * difference between one connection and three.
 */
export function createRefCount() {
  let held = 0;
  const watchers = new Set();

  function announce(wanted) {
    for (const watcher of [...watchers]) watcher(wanted);
  }

  return {
    /** How many requests are open right now. */
    count: () => held,
    /** Whether the stream should be open. */
    wanted: () => held > 0,
    /** Call `fn(wanted)` when the count crosses zero; returns the stop function. */
    watch(fn) {
      watchers.add(fn);
      return () => watchers.delete(fn);
    },
    /** One request for the shared stream; the returned function gives it back. */
    acquire() {
      const wasOpen = held > 0;
      held += 1;
      if (!wasOpen) announce(true);
      let open = true;
      return () => {
        if (!open) return;
        open = false;
        held = Math.max(0, held - 1);
        if (held === 0) announce(false);
      };
    },
  };
}

/** The payload a handler receives: an alert event carries the alert view of contract C9. */
export function payloadFor(name, data) {
  if (name === 'alert' && data && typeof data === 'object' && data.alert) return data.alert;
  return data;
}
