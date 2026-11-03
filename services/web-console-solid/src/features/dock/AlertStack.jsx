// AlertStack: mounted once by the Shell. Starts the shared alert polling, shows up to three
// pop-ups top-right (older ones live only in the dock's Alerts tab).
//
// Pop-ups are presentation only (U05): muting them renders nothing, and hiding one card removes
// it from this stack alone. Neither touches `alerts.open()`, so the bell keeps counting the
// refusals that are still waiting. Each card also leaves by itself after 12 s.
import { For, createSignal, createEffect, on, onCleanup } from 'solid-js';
import '../../styles/dock.css';
import AlertPopup from '../../components/AlertPopup.jsx';
import { navigate, paths } from '../../routing.js';
import { useUi } from '../../state/ui.jsx';
import { useAlerts } from './useAlerts.js';

/** How long one pop-up card stays before it hides itself, in milliseconds. */
export const HIDE_AFTER_MS = 12000;

export default function AlertStack() {
  const ui = useUi();
  const alerts = useAlerts({ run: true });
  const [fresh, setFresh] = createSignal(new Set());
  const shown = () =>
    ui.alerts.popups() === 'off'
      ? []
      : alerts.open().filter((alert) => !ui.alerts.isHidden(alert.id)).slice(0, 3);

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

  // Every visible card hides itself after 12 s; the timer dies with the card or the component.
  const hideTimers = new Map();
  createEffect(on(shown, (list) => {
    const visible = new Set(list.map((alert) => alert.id));
    for (const [id, timer] of hideTimers) {
      if (!visible.has(id)) {
        clearTimeout(timer);
        hideTimers.delete(id);
      }
    }
    for (const alert of list) {
      if (hideTimers.has(alert.id)) continue;
      const timer = setTimeout(() => {
        hideTimers.delete(alert.id);
        ui.alerts.hide(alert.id);
      }, HIDE_AFTER_MS);
      hideTimers.set(alert.id, timer);
    }
  }));
  onCleanup(() => {
    for (const timer of hideTimers.values()) clearTimeout(timer);
    hideTimers.clear();
  });

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
              onHide={() => ui.alerts.hide(alert.id)}
            />
          </div>
        )}
      </For>
    </div>
  );
}
