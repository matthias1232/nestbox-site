// The open alerts, shared by the bell, the pop-up stack and the Alerts tab.
// One module-level state; `useAlerts({ run: true })` (AlertStack only) starts the polling
// and the live subscription, everything else just reads.
import { createSignal, onCleanup } from 'solid-js';
import { usePolling } from '../../components/Poll.js';
import { useLive } from '../../state/live.jsx';
import { useUi } from '../../state/ui.jsx';
import * as alertsApi from '../../api/alerts.js';
import * as policies from '../../api/policies.js';
import { navigate, paths } from '../../routing.js';
import { allowBody, windowBody, draftEvent } from '../../components/alertAnswers.js';

const [all, setAll] = createSignal([]);
const [busy, setBusy] = createSignal({});
const [done, setDone] = createSignal({});

// Merge a live alert into the list: a known id adds its count, a new one goes first.
export function mergeLive(list, alert) {
  if (!alert || !alert.id) return list;
  const known = list.find((item) => item.id === alert.id);
  if (!known) return [alert, ...list];
  const merged = { ...known, ...alert, count: (Number(known.count) || 0) + (Number(alert.count) || 1) };
  return [merged, ...list.filter((item) => item.id !== alert.id)];
}

// Use the shared alerts. Pass `{ run: true }` once, in the component that is always mounted.
export function useAlerts({ run = false } = {}) {
  const ui = useUi();
  const live = useLive();

  const open = () => all().filter((alert) => !ui.alerts.isIgnored(alert.id));
  const sync = () => ui.alerts.setCount(open().length);

  if (run) {
    usePolling(() => alertsApi.list({ window: '15m' }), {
      everyMs: 15000,
      onData: (result) => { setAll(result.items || []); sync(); },
      onError: () => {},
    });
    const stop = live.on('alert', (alert) => { setAll((list) => mergeLive(list, alert)); sync(); });
    onCleanup(stop);
  }

  function leave(alert) {
    // The result shows for 2 s, then the card leaves the list.
    setTimeout(() => {
      setAll((list) => list.filter((item) => item.id !== alert.id));
      setDone((map) => { const next = { ...map }; delete next[alert.id]; return next; });
      sync();
    }, 2000);
  }

  // Answer one alert with an id from alertAnswers.answers().
  async function answer(alert, choice) {
    if (choice === 'ignore') { ui.alerts.ignore(alert.id); sync(); return; }
    if (choice === 'open') { navigate(paths.sandbox(alert.sandbox_id)); return; }
    setBusy((map) => ({ ...map, [alert.id]: true }));
    try {
      if (choice === 'now') {
        await policies.allowDestination(alert.sandbox_id, allowBody(alert));
        setDone((map) => ({ ...map, [alert.id]: `Allowed ${alert.host}` }));
        ui.toast.show({ tone: 'ok', text: `Allowed ${alert.host}:${alert.port}`, undo: null });
      } else if (choice === '15' || choice === '60') {
        const body = windowBody(alert, choice);
        const window = await policies.openWindow(body.sandbox_id, body.minutes, body.reason, body.mode, body.channel, false);
        setDone((map) => ({ ...map, [alert.id]: `Open for ${body.minutes} min` }));
        const windowId = window && window.id;
        ui.toast.show({
          tone: 'ok',
          text: `Allowed ${alert.host} for ${body.minutes} min`,
          undo: windowId ? () => policies.closeWindow(windowId) : null,
        });
      } else if (choice === 'draft') {
        await policies.draftFromEvent(draftEvent(alert), alert.sandbox_id);
        setDone((map) => ({ ...map, [alert.id]: 'Draft rule saved' }));
        ui.toast.show({ tone: 'info', text: `Draft rule for ${alert.host} saved`, undo: null });
      }
      leave(alert);
    } catch (failure) {
      ui.toast.show({ tone: 'bad', text: failure.message || 'The answer failed', undo: null });
    } finally {
      setBusy((map) => { const next = { ...map }; delete next[alert.id]; return next; });
    }
  }

  // Ignore every open alert until the page reloads.
  function ignoreAll() {
    for (const alert of open()) ui.alerts.ignore(alert.id);
    sync();
  }

  return { open, answer, ignoreAll, busy, done };
}
