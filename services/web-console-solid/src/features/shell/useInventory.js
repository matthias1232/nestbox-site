// The inventory tree for the sidebar and the home page (contract C3).
//
// One poll serves every subscriber: the first `useInventory()` call starts a 10 s timer, the
// last `onCleanup` stops it. Live events `sandbox`, `node`, `job` and `alert` trigger a refresh,
// debounced to 500 ms. F05, F06 and F17 code against `{ root, error, refresh, view, setView }`.
import { createEffect, createSignal, on, onCleanup } from 'solid-js';
import * as inventory from '../../api/inventory.js';
import { useLive } from '../../state/live.jsx';
import { useUi } from '../../state/ui.jsx';

const POLL_MS = 10000;
const DEBOUNCE_MS = 500;
const LIVE_EVENTS = ['sandbox', 'node', 'job', 'alert'];

const [root, setRoot] = createSignal(null);
const [error, setError] = createSignal(null);
const [localView, setLocalView] = createSignal('infrastructure');

let subscribers = 0;
let timer = null;
let debounce = null;
let sequence = 0;

// Load the tree of `view`; a slower, older answer never overwrites a newer one.
async function load(view) {
  sequence += 1;
  const mine = sequence;
  try {
    const answer = await inventory.tree(view);
    if (mine !== sequence) return;
    setRoot(answer?.root || null);
    setError(null);
  } catch (failure) {
    if (mine === sequence) setError(failure?.message || 'Could not load the inventory.');
  }
}

// Ask for a refresh soon; several events inside the window cost one request.
function refreshSoon(view) {
  if (debounce) clearTimeout(debounce);
  debounce = setTimeout(() => {
    debounce = null;
    load(view());
  }, DEBOUNCE_MS);
}

// The tree root (or null), the last error, a refresh function and the current view.
// Call it inside a component: it listens to the live stream and cleans up with the owner.
export function useInventory() {
  const ui = useUi();
  const live = useLive();
  // Without a UI store (a test, a pop-out) the module's own view signal is used.
  const view = ui ? ui.tree.view : localView;
  const setView = ui ? ui.tree.setView : setLocalView;

  subscribers += 1;
  if (subscribers === 1) timer = setInterval(() => load(view()), POLL_MS);

  // Load now and whenever the view changes.
  createEffect(on(view, (value) => load(value)));

  const stops = live ? LIVE_EVENTS.map((name) => live.on(name, () => refreshSoon(view))) : [];

  onCleanup(() => {
    stops.forEach((stop) => stop());
    subscribers -= 1;
    if (subscribers === 0) {
      if (timer) clearInterval(timer);
      if (debounce) clearTimeout(debounce);
      timer = null;
      debounce = null;
    }
  });

  return { root, error, refresh: () => load(view()), view, setView };
}
