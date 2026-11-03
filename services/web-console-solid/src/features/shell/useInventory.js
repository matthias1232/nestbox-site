// The inventory tree for the sidebar and the home page (contract C3).
//
// One poll serves every subscriber: the first `useInventory()` call starts a 10 s timer, the
// last `onCleanup` stops it. Live events `sandbox`, `node`, `job` and `alert` trigger a refresh,
// debounced to 500 ms. A load is skipped while the last good answer for the same view is fresh,
// so pages that mount together share one request; `refresh()` always loads.
//
// The server answers the tree for every data center the caller may see, not just the active
// one (that is the platform view). A switch of the topbar picker therefore changes the request
// header but not the answer, and the page kept showing the previous data center's sandboxes
// (U06-04). So the hook watches the active environment itself: a switch forgets the tree it
// showed, loads the new one, and `center()` names the subtree that belongs to the data center
// the user is in. Screens that mean "where I am" (sidebar, home, traffic) read `center()`;
// screens that resolve an id anywhere in the organisation (a link, a node page) read `root()`.
// F05, F06 and F17 code against `{ root, center, error, refresh, view, setView }`.
import { createEffect, createSignal, on, onCleanup } from 'solid-js';
import * as inventory from '../../api/inventory.js';
import { useLive } from '../../state/live.jsx';
import { useSession } from '../../state/session.jsx';
import { useUi } from '../../state/ui.jsx';

const POLL_MS = 10000;
const DEBOUNCE_MS = 500;
// An answer this young is still the answer: the sidebar, the home page and the data-center page
// mount within the same second, and each of them asked for the whole tree before.
const FRESH_MS = 2000;
const LIVE_EVENTS = ['sandbox', 'node', 'job', 'alert'];

const [root, setRoot] = createSignal(null);
const [error, setError] = createSignal(null);
const [localView, setLocalView] = createSignal('infrastructure');

let subscribers = 0;
let timer = null;
let debounce = null;
let sequence = 0;
let answeredKey = null;
let answeredAt = 0;
// The data center the module's signals belong to; `undefined` until the first caller names one
// (a test or a pop-out without a session has no data center, which is a value of its own).
let shownEnvironment;

// One answer is only good for the view and the data center it was asked for. A different pair
// is a different answer even when it arrived a moment ago.
function keyOf(view, environment) {
  return `${environment || ''}|${view}`;
}

// The session, or null outside a provider (a unit test, a pop-out that adopted its own state).
function optionalSession() {
  try {
    return useSession();
  } catch {
    return null;
  }
}

// Forget what the previous data center showed, so a switch never leaves its sandboxes on screen
// while the new request runs. An answer still in flight is dropped by the sequence bump.
function adoptEnvironment(environment) {
  const next = environment || null;
  if (shownEnvironment === next) return;
  shownEnvironment = next;
  sequence += 1;
  answeredKey = null;
  answeredAt = 0;
  setRoot(null);
  setError(null);
}

// The data center node an inventory tree holds for `id`, or null while it is not in the tree.
function centerOf(tree, id) {
  if (!tree || !id) return null;
  let found = null;
  (function walk(node) {
    if (!node || found) return;
    if (node.kind === 'data_center' && String(node.id) === String(id)) {
      found = node;
      return;
    }
    (node.children || []).forEach(walk);
  })(tree);
  return found;
}

// Load the tree of `view` for `environment`; a slower, older answer never overwrites a newer
// one. Unless `force` is set, an answer for the same pair that arrived less than `FRESH_MS`
// ago is left alone.
async function load(view, environment, { force = false } = {}) {
  adoptEnvironment(environment);
  const key = keyOf(view, environment);
  if (!force && answeredKey === key && answeredAt && Date.now() - answeredAt < FRESH_MS) return;
  sequence += 1;
  const mine = sequence;
  try {
    const answer = await inventory.tree(view);
    if (mine !== sequence) return;
    setRoot(answer?.root || null);
    setError(null);
    answeredKey = key;
    answeredAt = Date.now();
  } catch (failure) {
    if (mine === sequence) setError(failure?.message || 'Could not load the inventory.');
  }
}

// Ask for a refresh soon; several events inside the window cost one request.
function refreshSoon(view, environment) {
  if (debounce) clearTimeout(debounce);
  debounce = setTimeout(() => {
    debounce = null;
    load(view(), environment());
  }, DEBOUNCE_MS);
}

// The tree root (or null), the active data center's subtree, the last error, a refresh function
// and the current view. Call it inside a component: it listens to the live stream and cleans up
// with the owner.
export function useInventory() {
  const ui = useUi();
  const live = useLive();
  const session = optionalSession();
  // Without a UI store (a test, a pop-out) the module's own view signal is used.
  const view = ui ? ui.tree.view : localView;
  const setView = ui ? ui.tree.setView : setLocalView;
  // Without a session there is no data center to scope by; the whole tree is the answer.
  const environment = () => (session ? session.activeEnvironment() : null);

  subscribers += 1;
  if (subscribers === 1) timer = setInterval(() => load(view(), environment()), POLL_MS);

  // Load now, on a view change and on a data-center switch: the second changes the header the
  // request carries, so the answer worth having is a new one.
  createEffect(on(() => [view(), environment()], ([value, center]) => load(value, center)));

  const stops = live ? LIVE_EVENTS.map((name) => live.on(name, () => refreshSoon(view, environment))) : [];

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

  return {
    root,
    // The active data center's subtree. A view with no data-center level (models hang off the
    // organisation) keeps the whole tree rather than an empty sidebar.
    center: () => centerOf(root(), environment()) || root(),
    error,
    refresh: () => load(view(), environment(), { force: true }),
    view,
    setView,
  };
}
