// A bounded list of live events, keyed so a later event updates its row.
//
// The ticker is deliberately not reactive: it is the data under a screen, and
// the screen decides how to publish a change. Every read returns items newest
// first, so a caller can prepend without thinking about event order.

function timestamp(value) {
  if (value == null) return 0;
  if (typeof value === 'number') return Number.isFinite(value) ? value : 0;
  const parsed = Date.parse(String(value));
  return Number.isNaN(parsed) ? 0 : parsed;
}

/**
 * Create a ticker holding at most `max` events.
 *
 * `upsert(event, key)` merges a matching event with `{...old, ...new}` and moves
 * it to its place in the time order, so a later event with the same key updates
 * a row instead of duplicating it. `remove(key)` drops one event, `clear()` all.
 */
export function createTicker({ max = 300 } = {}) {
  let items = [];
  const bound = Number.isFinite(max) && max >= 0 ? max : 300;

  const sortNewestFirst = (list) =>
    [...list].sort((left, right) => timestamp(right?.ts) - timestamp(left?.ts));

  return {
    items: () => items,

    upsert(event, key = (item) => item?.id) {
      const id = key(event);
      const existing = items.find((item) => key(item) === id);
      const merged = existing ? { ...existing, ...event } : event;
      items = sortNewestFirst([merged, ...items.filter((item) => key(item) !== id)]).slice(
        0,
        bound,
      );
      return merged;
    },

    remove(key) {
      const matches = typeof key === 'function' ? key : (item) => item?.id === key;
      items = items.filter((item) => !matches(item));
    },

    clear() {
      items = [];
    },
  };
}
