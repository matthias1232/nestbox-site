// Alerts tab of the dock: every open refused destination as a row with its answers.
import { For, Show } from 'solid-js';
import AlertPopup from '../../components/AlertPopup.jsx';
import { navigate, paths } from '../../routing.js';
import { useAlerts } from './useAlerts.js';

export default function AlertsTab() {
  const alerts = useAlerts();
  return (
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
  );
}
