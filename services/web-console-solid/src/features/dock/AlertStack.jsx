// AlertStack: mounted once by the Shell. Starts the shared alert polling, shows up to three
// pop-ups top-right (older ones live only in the dock's Alerts tab).
import { For, createSignal, createEffect, on } from 'solid-js';
import '../../styles/dock.css';
import AlertPopup from '../../components/AlertPopup.jsx';
import { navigate, paths } from '../../routing.js';
import { useAlerts } from './useAlerts.js';

export default function AlertStack() {
  const alerts = useAlerts({ run: true });
  const [fresh, setFresh] = createSignal(new Set());
  const shown = () => alerts.open().slice(0, 3);

  // A newly seen alert plays the 600 ms flash once.
  let seen = new Set();
  createEffect(on(shown, (list) => {
    const added = list.filter((a) => !seen.has(a.id)).map((a) => a.id);
    seen = new Set(list.map((a) => a.id));
    if (added.length && seen.size > added.length) {
      setFresh(new Set(added));
      setTimeout(() => setFresh(new Set()), 700);
    }
  }));

  return (
    <div class="alert-stack" aria-live="polite">
      <For each={shown()}>
        {(alert) => (
          <div classList={{ 'alert-flash': fresh().has(alert.id) }}>
            <AlertPopup
              alert={alert}
              busy={!!alerts.busy()[alert.id]}
              result={alerts.done()[alert.id]}
              onAllowNow={() => alerts.answer(alert, 'now')}
              onAllowFor={(minutes) => alerts.answer(alert, String(minutes))}
              onDraft={() => alerts.answer(alert, 'draft')}
              onIgnore={() => alerts.answer(alert, 'ignore')}
              onOpenSandbox={() => navigate(paths.sandbox(alert.sandbox_id))}
            />
          </div>
        )}
      </For>
    </div>
  );
}
