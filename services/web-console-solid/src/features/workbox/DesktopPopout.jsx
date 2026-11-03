// The desktop in a window of its own.
//
// A desktop is watched the way a television is watched: from a distance, often
// while something else is on the screen. The pop-out is the same stream as the
// panel, opened by a person who wants the whole window for it.
//
// Watching without touching is the server's decision (`?view=1`), set by the
// checkbox in the panel; this window offers the same control.

import { Show, createEffect, createResource, createSignal, lazy } from 'solid-js';
import * as sandboxes from '../../api/sandboxes.js';
import * as presence from '../../api/presence.js';
import { usePolling } from '../../components/Poll.js';
import PresenceDots from '../../components/PresenceDots.jsx';
const DesktopTab = lazy(() => import('../sandboxes/DesktopTab.jsx'));
import '../../styles/workbox-sessions.css';

export default function DesktopPopout(props) {
  const [sandbox] = createResource(() => props.sandboxId, sandboxes.get);
  const [viewers, setViewers] = createSignal([]);

  usePolling(() => presence.presence(props.sandboxId), {
    everyMs: 10000,
    onData: (answer) => {
      // Every desktop viewer of this sandbox, whoever's window they sit in.
      const items = Array.isArray(answer) ? answer : answer?.items || [];
      setViewers(items.filter((entry) => entry.kind === 'desktop'));
    },
  });

  createEffect(() => {
    const name = sandbox()?.name;
    document.title = name ? `${name} · desktop` : 'Desktop';
  });

  const people = () =>
    viewers().map((entry) => ({
      name: entry.user_name || 'someone',
      kind: entry.kind,
      view_only: entry.view_only === true,
    }));

  return (
    <div class="ws-popout">
      <header class="ws-titlebar">
        <h1>{sandbox()?.name || props.sandboxId}</h1>
        <span class="muted small">desktop</span>
        <Show when={viewers().length > 0}>
          <PresenceDots people={people()} />
          <span class="muted small">
            {viewers().map((entry) => entry.user_name || 'someone').join(', ')}
          </span>
        </Show>
      </header>
      <div class="ws-popout-body">
        <Show
          when={sandbox()}
          fallback={
            <Show when={sandbox.error} fallback={<p class="muted">Opening the desktop …</p>}>
              <p class="error">{sandbox.error?.message || 'This sandbox could not be read.'}</p>
            </Show>
          }
        >
          {(value) => <DesktopTab sandbox={value()} fullWindow />}
        </Show>
      </div>
    </div>
  );
}
