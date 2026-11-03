// Turn details, fetched at most a few at a time.
//
// A thread holds hundreds of turns and the findings of each one live behind its
// own request. Scrolling must not open a burst of them, so the queue keeps four
// in flight and remembers what it already has. The queue is a factory, not a
// module-level singleton, so a test can drive it with a fake loader.

/** How many turn details may be in flight at once. */
export const MAX_IN_FLIGHT = 4;

/**
 * Build a bounded loader.
 *
 * `load(id)` is the real fetch. A failure resolves to `null`: a finding that
 * could not be read is shown as "not loaded", not as an error that hides the
 * turn's text, which is the part the reader came for.
 */
export function createFindingsQueue({ load, max = MAX_IN_FLIGHT } = {}) {
  const cache = new Map();
  const inflight = new Map();
  const waiting = [];
  let active = 0;

  function pump() {
    while (active < max && waiting.length) {
      const job = waiting.shift();
      active += 1;
      Promise.resolve()
        .then(() => load(job.id))
        .then(
          (value) => {
            cache.set(job.id, value);
            job.resolve(value);
          },
          () => job.resolve(null),
        )
        .then(() => {
          active -= 1;
          inflight.delete(job.id);
          pump();
        });
    }
  }

  return {
    /** The details of one turn, from the cache, the queue or the network. */
    loadTurn(id) {
      if (!id) return Promise.resolve(null);
      if (cache.has(id)) return Promise.resolve(cache.get(id));
      if (inflight.has(id)) return inflight.get(id);
      const promise = new Promise((resolve) => {
        waiting.push({ id, resolve });
        pump();
      });
      inflight.set(id, promise);
      return promise;
    },
    /** What is known already, without asking for anything. */
    cached(id) {
      return cache.get(id);
    },
    /** How many requests are running: the queue's bound, made observable. */
    active() {
      return active;
    },
    /** How many are still waiting for a slot. */
    waiting() {
      return waiting.length;
    },
  };
}
