// State for the chat browser: the grouped list, the selected thread, its
// filters, the older pages, and the live stream folded on top.
//
// No JSX here (`features/AGENTS.md`): this file decides what a filter means,
// which page comes next, and how an event becomes a turn; the browser renders.
//
// A selection is a group, not a sandbox: the same component is used for one
// sandbox (where the sandbox is fixed) and for the whole environment, so
// everything that is not fixed arrives as a filter.

import { createEffect, createMemo, createSignal, onCleanup, untrack } from 'solid-js';
import * as chatsApi from '../../api/chats.js';
import { EVENT, applyEvent, emptyState, mergeLive, replaceTurn, sortTurns } from './chatsMerge.js';

/** How the list may be grouped. `by` is the server's word for it. */
export const GROUPINGS = [
  { value: 'session', label: 'Session' },
  { value: 'model', label: 'Modell' },
  { value: 'day', label: 'Tag' },
  { value: 'sandbox', label: 'Sandbox' },
  { value: 'owner', label: 'Nutzer' },
];

/** The order of the group list. A thread itself always reads oldest first. */
export const ORDERS = [
  { value: 'newest', label: 'neueste' },
  { value: 'oldest', label: 'älteste' },
];

/** The content-check filter, in the server's states. */
export const DLP_STATES = [
  { value: '', label: 'all checks' },
  { value: 'clean', label: 'sauber' },
  { value: 'blocked', label: 'blockiert' },
  { value: 'replaced', label: 'ersetzt' },
  { value: 'pseudonymized', label: 'pseudonymisiert' },
];

/** Which account paid. The two the server records. */
export const ACCOUNT_KINDS = [
  { value: '', label: 'alle Konten' },
  { value: 'subscription', label: 'Abo' },
  { value: 'api_key', label: 'API key' },
];

/** Turns per page. Large enough to fill a thread, small enough to arrive fast. */
export const PAGE = 50;

/** The search field waits for the typing to stop before it asks the server. */
export const DEBOUNCE_MS = 400;

/** The turn filters one group stands for. */
export function groupParams(by, key) {
  if (!key) return {};
  if (by === 'session') return { session_id: key };
  if (by === 'model') return { model: key };
  if (by === 'provider') return { provider: key };
  if (by === 'sandbox') return { sandbox_id: key };
  if (by === 'owner') return { owner: key };
  if (by === 'day') {
    if (/^\d{4}-\d{2}-\d{2}$/.test(key)) {
      return { since: `${key}T00:00:00Z`, until: `${key}T23:59:59Z` };
    }
    return { since: key };
  }
  return {};
}

function trimmed(value) {
  return String(value ?? '').trim();
}

/**
 * The chat state.
 *
 * `options.sandboxId` is an accessor, so a drawer that switches sandbox does not
 * need a new hook instance.
 */
export function useChats(options = {}) {
  const sandboxId = () => options.sandboxId?.() || null;

  const [by, setBy] = createSignal('session');
  const [order, setOrder] = createSignal('newest');
  const [search, setSearch] = createSignal('');
  const [q, setQ] = createSignal('');
  const [model, setModel] = createSignal('');
  const [provider, setProvider] = createSignal('');
  const [dlp, setDlp] = createSignal('');
  const [accountKind, setAccountKind] = createSignal('');

  const [groups, setGroups] = createSignal([]);
  const [selected, setSelected] = createSignal(null);
  const [turns, setTurns] = createSignal([]);
  const [loading, setLoading] = createSignal(false);
  const [loadingOlder, setLoadingOlder] = createSignal(false);
  const [hasMore, setHasMore] = createSignal(false);
  const [error, setError] = createSignal(null);

  const [liveEnabled, setLiveEnabled] = createSignal(true);
  const [live, setLive] = createSignal(emptyState());
  const [lastEventAt, setLastEventAt] = createSignal(null);

  const filters = createMemo(() => {
    const out = {};
    if (q()) out.q = q();
    if (trimmed(model())) out.model = trimmed(model());
    if (trimmed(provider())) out.provider = trimmed(provider());
    if (dlp()) out.dlp = dlp();
    if (accountKind()) out.account_kind = accountKind();
    return out;
  });

  const turnParams = createMemo(() => {
    const base = { ...filters() };
    if (sandboxId()) base.sandbox_id = sandboxId();
    return { ...base, ...groupParams(by(), selected()?.key) };
  });

  /** The groupings this view offers: a sandbox view cannot group by sandbox. */
  const groupings = createMemo(() =>
    GROUPINGS.filter((option) =>
      sandboxId() ? option.value !== 'sandbox' && option.value !== 'owner' : true,
    ),
  );

  const sortedGroups = createMemo(() => {
    const rows = [...groups()];
    const sign = order() === 'oldest' ? 1 : -1;
    rows.sort((left, right) => (Date.parse(left.last_ts) - Date.parse(right.last_ts)) * sign);
    return rows;
  });

  /** What the thread shows: the stored page with the running turns on top. */
  const visibleTurns = createMemo(() => mergeLive(turns(), live().turns));

  const title = createMemo(() => {
    const group = selected();
    if (!group) return 'Alle, chronologisch';
    return group.label || group.key;
  });

  let searchTimer = null;
  let groupToken = 0;
  let turnToken = 0;

  function changeSearch(value) {
    setSearch(value);
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => setQ(trimmed(value)), DEBOUNCE_MS);
  }

  async function loadGroups() {
    const grouping = untrack(by);
    // A view that is pinned to one sandbox cannot group by sandbox or owner;
    // a stale choice from the environment view falls back rather than erroring.
    if (untrack(sandboxId) && (grouping === 'sandbox' || grouping === 'owner')) {
      setBy('session');
      return;
    }
    const token = ++groupToken;
    setLoading(true);
    const params = { by: grouping, ...untrack(filters) };
    const pinned = untrack(sandboxId);
    if (pinned) params.sandbox_id = pinned;
    try {
      const rows = await chatsApi.groups(params);
      if (token !== groupToken) return;
      const list = Array.isArray(rows) ? rows : [];
      setGroups(list);
      setError(null);
      const current = untrack(selected);
      if (current) {
        const still = list.find((group) => group.key === current.key);
        // A group whose last turn fell out of the filter is gone; the thread
        // then shows everything instead of an empty page under a stale heading.
        setSelected(still || null);
      }
    } catch (failure) {
      if (token === groupToken) setError(failure?.message || 'Die Chatliste konnte nicht geladen werden.');
    } finally {
      if (token === groupToken) setLoading(false);
    }
  }

  async function loadTurns({ append = false } = {}) {
    const token = ++turnToken;
    const params = { ...untrack(turnParams), limit: PAGE };
    if (append) {
      const oldest = untrack(turns).at(-1);
      if (!oldest?.ts) return;
      params.before = oldest.ts;
      setLoadingOlder(true);
    } else {
      setLoading(true);
    }
    try {
      const rows = await chatsApi.turns(params);
      if (token !== turnToken) return;
      const list = Array.isArray(rows) ? rows : [];
      setTurns((current) => (append ? [...current, ...list] : list));
      setHasMore(list.length >= PAGE);
      setError(null);
    } catch (failure) {
      if (token === turnToken) setError(failure?.message || 'Die Anfragen konnten nicht geladen werden.');
    } finally {
      if (token === turnToken) {
        setLoading(false);
        setLoadingOlder(false);
      }
    }
  }

  /**
   * Fold one stream event in.
   *
   * A sandbox view drops events of other sandboxes, because a stream that says
   * everything must not put another sandbox's prompt on this screen.
   */
  function push(event) {
    const pinned = untrack(sandboxId);
    if (pinned && event?.sandbox_id && event.sandbox_id !== pinned) return;
    setLive((state) => applyEvent(state, event));
    setLastEventAt(event?.ts || new Date().toISOString());
  }

  /** Replace the running turn with the stored one once the call has ended. */
  async function loadDetail(id) {
    try {
      const detail = await chatsApi.turn(id);
      const stored = detail?.turn;
      if (!stored) return;
      setLive((state) => replaceTurn(state, { ...stored, incidents: detail?.incidents || [] }));
    } catch {
      // The running turn keeps growing from the stream; the next list load
      // replaces it. A failed detail read is not worth an error banner.
    }
  }

  createEffect(() => {
    by();
    filters();
    sandboxId();
    untrack(loadGroups);
  });

  createEffect(() => {
    by();
    filters();
    sandboxId();
    selected();
    untrack(loadTurns);
  });

  createEffect(() => {
    if (!liveEnabled()) return;
    const stop = chatsApi.stream({
      onChatStarted: (event) => push({ ...event, type: EVENT.started }),
      onChatDelta: (event) => push({ ...event, type: EVENT.delta }),
      onChatDone: (event) => {
        push({ ...event, type: EVENT.done });
        if (event?.turn_id) loadDetail(event.turn_id);
      },
    });
    onCleanup(stop);
  });

  onCleanup(() => {
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = null;
  });

  async function refresh() {
    await loadGroups();
    await loadTurns();
  }

  /** The visible groups, in the chosen order. */
  function select(group) {
    setSelected(group || null);
  }

  function loadOlder() {
    if (loadingOlder()) return;
    return loadTurns({ append: true });
  }

  return {
    // list
    by,
    setBy,
    groupings,
    order,
    setOrder,
    groups: sortedGroups,
    loading,
    error,
    select,
    selected,
    // filters
    search,
    changeSearch,
    model,
    setModel,
    provider,
    setProvider,
    dlp,
    setDlp,
    accountKind,
    setAccountKind,
    // thread
    turns: visibleTurns,
    orderedTurns: () => sortTurns(visibleTurns(), 'asc'),
    title,
    hasMore,
    loadingOlder,
    loadOlder,
    refresh,
    // live
    liveEnabled,
    setLiveEnabled,
    lastEventAt,
    pendingCount: () => live().turns.filter((turn) => turn.pending).length,
  };
}
