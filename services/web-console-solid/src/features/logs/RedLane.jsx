// The red lane: refused destinations of the shown sources, with the same answers as the dock.
// More than three alerts collapse to "N refused" until opened. An answered alert leaves the lane.
import { For, Show, createSignal } from 'solid-js';
import AlertPopup from '../../components/AlertPopup.jsx';
import { useAlerts } from '../dock/useAlerts.js';
import { navigate, paths } from '../../routing.js';

// props: sandboxIds (array of ids, or null for every sandbox), run (start polling; pop-out only),
// inWindow (do not navigate this window away)
export default function RedLane(props) {
  const alerts = useAlerts({ run: !!props.run });
  const [expanded, setExpanded] = createSignal(false);
  const shown = () => alerts.open().filter((a) => !props.sandboxIds || props.sandboxIds.includes(a.sandbox_id));
  const visible = () => (shown().length > 3 && !expanded() ? [] : shown());
  const openSandbox = (alert) => { if (!props.inWindow) navigate(paths.sandbox(alert.sandbox_id)); };

  return (
    <Show when={shown().length > 0}>
      <section class="red-lane" role="region" aria-label="Refused connections">
        <header class="red-lane-head">
          <span class="badge bad"><i />{shown().length} refused</span>
          <Show when={shown().length > 3}>
            <button class="ghost tiny" aria-expanded={expanded()} onClick={() => setExpanded(!expanded())}>
              {expanded() ? 'Hide' : 'Show'}
            </button>
          </Show>
          <button class="ghost tiny" onClick={() => alerts.ignoreAll()}>Ignore all for this session</button>
        </header>
        <Show when={visible().length > 0}>
          <table class="dock-table">
            <thead><tr><th>Destination</th><th>Sandbox</th><th>Channel</th><th>Seen</th><th>Answer</th></tr></thead>
            <tbody>
              <For each={visible()}>
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
                    onOpenSandbox={() => openSandbox(alert)}
                  />
                )}
              </For>
            </tbody>
          </table>
        </Show>
      </section>
    </Show>
  );
}
