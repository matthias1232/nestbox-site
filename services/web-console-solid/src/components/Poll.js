// Poll a loader while a component is mounted. `load` returns a promise; each
// answer goes to `onData`, a failure to `onError`. Pausing stops the timer, and
// so does a hidden tab: to save work, no request leaves a window nobody looks at.
import { createSignal, onCleanup, onMount } from 'solid-js';

// Whether the timer and its requests should run. Hidden wins over the user's
// choice, so a tab that comes back resumes into the state it was left in.
export function pollActive({ hidden, paused }) {
  return !hidden && !paused;
}

export function usePolling(load, { everyMs = 5000, onData, onError } = {}) {
  const [paused, setPaused] = createSignal(false);
  let timer = null;
  let alive = true;

  const hidden = () => typeof document !== 'undefined' && Boolean(document.hidden);

  async function tick() {
    if (!alive) return;
    try {
      const data = await load();
      if (alive) onData?.(data);
    } catch (failure) {
      if (alive) onError?.(failure);
    }
  }

  function start() {
    stop();
    if (pollActive({ hidden: hidden(), paused: paused() })) timer = setInterval(tick, everyMs);
  }
  function stop() {
    if (timer) clearInterval(timer);
    timer = null;
  }

  // A hidden tab stops; a visible one starts again and answers at once, because
  // whatever it shows is as old as the time it was away.
  function onVisibility() {
    if (!alive) return;
    if (!pollActive({ hidden: hidden(), paused: paused() })) {
      stop();
      return;
    }
    tick();
    start();
  }

  onMount(() => {
    tick();
    start();
    if (typeof document !== 'undefined') document.addEventListener('visibilitychange', onVisibility);
  });
  onCleanup(() => {
    alive = false;
    stop();
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', onVisibility);
  });

  return {
    refresh: tick,
    paused,
    toggle() {
      setPaused(!paused());
      paused() ? stop() : start();
    },
  };
}
