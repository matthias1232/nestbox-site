// Alerts tab of the dock: every open refused destination as a row with its answers.
//
// The head line names the pop-up state (on / muted until 15:40 / muted) with a toggle, because
// the tab is what is left when the pop-ups are silent: the rows keep listing every open alert,
// muted or not, and the bell keeps its count.
import { For, Show } from 'solid-js';
import AlertPopup from '../../components/AlertPopup.jsx';
import { navigate, paths } from '../../routing.js';
import { useUi } from '../../state/ui.jsx';
import { useAlerts } from './useAlerts.js';

/** The clock time of a timed mute, or '' when the moment is missing. */
function clockTime(ms) {
  if (!Number.isFinite(ms)) return '';
  return new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

export default function AlertsTab() {
  const ui = useUi();
  const alerts = useAlerts();
  const muted = () => ui.alerts.popups() === 'off';
  const state = () => {
    if (!muted()) return 'on';
    const until = ui.alerts.popupEnds();
    return until ? `muted until ${clockTime(until)}` : 'muted';
  };
  return (
    <>
      <div class="dock-tools alert-popup-state">
        <span class="muted small">Pop-ups: <span class="mono">{state()}</span></span>
        <button class="ghost tiny" onClick={() => ui.alerts.setPopups(muted() ? 'on' : 'off')}>
          {muted() ? 'Turn pop-ups on' : 'Mute pop-ups'}
        </button>
      </div>
      <Show
        when={alerts.open().length}
        fallback={<p class="muted">No refused connections are waiting. Blocked destinations appear here and pop up top-right.</p>}
      >
        <div class="dock-tools">
          <span class="muted small">{alerts.open().length} open</span>
          <button class="ghost tiny" onClick={() => alerts.ignoreAll()}>Ignore all for this session</button>
        </div>
        <table class="dock-table">
          <thead><tr><th>Destination</th><th>Sandbox</th><th>Channel</th><th>Seen</th><th>Answer</th></tr></thead>
          <tbody>
            <For each={alerts.open()}>
              {(alert) => (
                <AlertPopup
                  inline
                  alert={alert}
                  busy={!!alerts.busy()[alert.id]}
                  result={alerts.done()[alert.id]}
                  onAllowNow={() => alerts.answer(alert, 'now')}
                  onAllowFor={(minutes) => alerts.answer(alert, String(minutes))}
                  onDraft={() => alerts.answer(alert, 'draft')}
                  onIgnore={() => alerts.answer(alert, 'ignore')}
                  onOpenSandbox={() => navigate(paths.sandbox(alert.sandbox_id))}
                />
              )}
            </For>
          </tbody>
        </table>
      </Show>
    </>
  );
}
