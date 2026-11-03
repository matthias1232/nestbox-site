// Data hooks for the cluster pages: a 10 s poll plus a refresh on every `models` live event.
import { createSignal, onCleanup } from 'solid-js';
import { usePolling } from '../../components/Poll.js';
import { useLive } from '../../state/live.jsx';
import * as clusters from '../../api/clusters.js';

// Subscribe a refresh function to the `models` event; returns nothing, cleans up itself.
function refreshOnModels(refresh) {
  const live = useLive();
  if (!live) return;
  const stop = live.on('models', () => refresh());
  onCleanup(stop);
}

// Every cluster, refreshed every 10 s and on `models` events.
export function useClusterList() {
  const [items, setItems] = createSignal([]);
  const [error, setError] = createSignal(null);
  const [loaded, setLoaded] = createSignal(false);
  const poll = usePolling(() => clusters.list(), {
    everyMs: 10000,
    onData: (answer) => { setItems(answer.items || []); setError(null); setLoaded(true); },
    onError: (failure) => { setError(failure.message); setLoaded(true); },
  });
  refreshOnModels(poll.refresh);
  return { items, error, loaded, refresh: poll.refresh };
}

// One cluster with its members, refreshed every 10 s and on `models` events.
export function useCluster(id) {
  const [cluster, setCluster] = createSignal(null);
  const [error, setError] = createSignal(null);
  const poll = usePolling(() => clusters.get(id()), {
    everyMs: 10000,
    onData: (value) => { setCluster(value); setError(null); },
    onError: (failure) => setError(failure.message),
  });
  refreshOnModels(poll.refresh);
  return { cluster, error, refresh: poll.refresh };
}
