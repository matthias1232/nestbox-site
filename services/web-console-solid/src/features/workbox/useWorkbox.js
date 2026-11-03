// The two reads the Workbox needs, refreshed on a timer and on live events.
//
// One hook rather than one per card: the header, the three cards and the panels
// all talk about the same sandbox, and a second poll would be a second truth
// about it. A card that changes something calls `refresh`, so the page reloads
// as one picture instead of half of it.

import { createMemo, createSignal, onCleanup } from 'solid-js';
import { usePolling } from '../../components/Poll.js';
import * as controlApi from '../../api/control.js';
import * as sandboxApi from '../../api/sandboxes.js';
import { onControl } from './useLive.js';

// A burst of events (a switch and the restart it triggers) costs one reload.
const DEBOUNCE_MS = 300;

/**
 * The sandbox row and its control summary, kept current.
 *
 * `sandboxId` is an accessor so the live stream and the polling loader can
 * re-read it. Returns `{ sandbox, control, error, refresh }`: `sandbox` is the
 * object `sandboxes.get` answers (the same shape the Admin Center tabs take),
 * `control` the summary of contract C9, `error` the first failure of either
 * read, and `refresh` reloads both at once.
 */
export function useWorkbox(sandboxId) {
  const [sandbox, setSandbox] = createSignal(null);
  const [summary, setSummary] = createSignal(null);
  const [sandboxError, setSandboxError] = createSignal(null);
  const [controlError, setControlError] = createSignal(null);

  // The row is what the header shows; it is also what tells a stale page from a
  // stopped sandbox after the container was removed underneath it.
  const row = usePolling(() => sandboxApi.get(sandboxId()), {
    everyMs: 15000,
    onData: (value) => {
      setSandbox(value);
      setSandboxError(null);
    },
    onError: (failure) => setSandboxError(failure.message),
  });

  // The summary carries agents, windows, people and models; it survives a
  // failure of the row, because a stopped sandbox still has a mode and a node.
  const controls = usePolling(() => controlApi.summary(sandboxId()), {
    everyMs: 15000,
    onData: (value) => {
      setSummary(value);
      setControlError(null);
    },
    onError: (failure) => setControlError(failure.message),
  });

  async function refresh() {
    await Promise.all([row.refresh(), controls.refresh()]);
  }

  // Someone else's change arrives as an event; waiting for the burst to end
  // means the reload sees the finished state and not the first step of it.
  let timer = null;
  const stop = onControl(sandboxId, () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      refresh();
    }, DEBOUNCE_MS);
  });

  onCleanup(() => {
    if (timer) clearTimeout(timer);
    timer = null;
    stop();
  });

  return {
    sandbox,
    control: summary,
    error: createMemo(() => sandboxError() || controlError()),
    refresh,
  };
}

/**
 * The sandboxes the caller can open, with the cheap per-sandbox overview.
 *
 * The list and the overview are two calls on purpose: the overview is one row
 * per sandbox and may fail or lag, and the list of names is still worth showing
 * without it. Returns `{ items, summaryOf, loading, error, load }`.
 */
export function useMyBoxes() {
  const [items, setItems] = createSignal([]);
  const [summaries, setSummaries] = createSignal({});
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal(null);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const page = await sandboxApi.list();
      setItems(Array.isArray(page?.items) ? page.items : []);
    } catch (failure) {
      setItems([]);
      setError(failure.message);
    } finally {
      setLoading(false);
    }
    try {
      const rows = await controlApi.overview();
      setSummaries(
        Object.fromEntries(
          (Array.isArray(rows) ? rows : []).map((row) => [row.sandbox_id, row]),
        ),
      );
    } catch {
      // The cards stay without the loud badge; a missing overview must not hide
      // the sandboxes the user can still open.
      setSummaries({});
    }
  }

  return {
    items,
    summaryOf: (id) => summaries()[id] || null,
    loading,
    error,
    load,
  };
}

/**
 * Split a list into "mine" and "everyone else's".
 *
 * Only an operator or an admin gets the split: a member's list is short and the
 * separation would add a heading without adding an answer. Returns `null` when
 * the caller should see one plain list.
 */
export function splitByOwner(list, identity) {
  if (!identity || !identity.user_id || identity.role === 'member') return null;
  return {
    mine: list.filter((item) => item.owner_user_id === identity.user_id),
    others: list.filter((item) => item.owner_user_id !== identity.user_id),
  };
}
