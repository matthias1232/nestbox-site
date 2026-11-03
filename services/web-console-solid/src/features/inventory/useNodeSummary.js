// The summary of one node (contract C8), refreshed every 5 s.
import { createEffect, createSignal, onCleanup } from 'solid-js';
import * as nodeOps from '../../api/nodeOps.js';
import { usePolling } from '../../components/Poll.js';
import { navigate } from '../../routing.js';
import { activeTab, tabFromSearch, tabPath, tabsFor } from './inventoryTabs.js';

// `nodeId` is a function returning the id. Returns `{ data, error, refresh }`.
export function useNodeSummary(nodeId) {
  const [data, setData] = createSignal(null);
  const [error, setError] = createSignal(null);
  const poll = usePolling(() => nodeOps.summary(nodeId()), {
    everyMs: 5000,
    onData: (value) => { setData(value); setError(null); },
    onError: (failure) => setError(failure.message || 'Could not read the node.'),
  });
  return { data, error, refresh: poll.refresh };
}

// The tab state of an inventory page: starts from `getTab()` (the `tab` prop or the address),
// follows the address when it changes, and `select(tab)` writes `?tab=` through `navigate`.
export function useTabState(kind, getRole, getTab, basePath) {
  const read = () => activeTab(kind, getRole(), getTab() || tabFromSearch(window.location.search));
  const [tab, setTab] = createSignal(read());
  const sync = () => setTab(read());
  window.addEventListener('popstate', sync);
  onCleanup(() => window.removeEventListener('popstate', sync));
  createEffect(() => {
    getTab();
    getRole();
    sync();
  });
  return {
    tab,
    tabs: () => tabsFor(kind, getRole()),
    select(next) {
      setTab(next);
      navigate(tabPath(basePath(), next));
    },
  };
}
