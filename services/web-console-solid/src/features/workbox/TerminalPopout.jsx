// One terminal tab in a window of its own.
//
// The Workbox is a page with cards; a terminal wants the whole screen. The tab
// is the same tmux session either way, so this window joins the shell another
// viewer may already be in and shows who else is there.
//
// The pop-out shares its opener's CSRF token (the shell does that on load), so
// closing a tab for everyone works here too.

import { Show, createEffect, createResource, createSignal, lazy } from 'solid-js';
import * as sandboxes from '../../api/sandboxes.js';
import * as presence from '../../api/presence.js';
import { usePolling } from '../../components/Poll.js';
import PresenceDots from '../../components/PresenceDots.jsx';
const TerminalTab = lazy(() => import('../sandboxes/TerminalTab.jsx'));
import '../../styles/workbox-sessions.css';

export default function TerminalPopout(props) {
  const [sandbox] = createResource(() => props.sandboxId, sandboxes.get);
  const [viewers, setViewers] = createSignal([]);

  usePolling(() => presence.presence(props.sandboxId), {
    everyMs: 10000,
    onData: (answer) => {
      // Everybody connected to the sandbox; this window only cares about its own
      // tab. A plain list is the contract, the `items` form is tolerated.
      const items = Array.isArray(answer) ? answer : answer?.items || [];
      setViewers(items.filter((entry) => entry.tab_id === props.tab));
    },
  });

  // The window title is the only handle a pop-out has in the task bar.
  createEffect(() => {
    const name = sandbox()?.name;
    document.title = name ? `${name} · ${props.tab}` : `Terminal ${props.tab}`;
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
        <span class="mono small">{props.tab}</span>
        <Show when={viewers().length > 0}>
          <PresenceDots people={people()} />
          <span class="muted small">
            {viewers().map((entry) => entry.user_name || 'someone').join(', ')}
          </span>
        </Show>
        <span class="muted small">This window keeps its own connection; the shell is shared with everyone in this tab.</span>
      </header>
      <div class="ws-popout-body">
        <Show
          when={sandbox()}
          fallback={
            <Show when={sandbox.error} fallback={<p class="muted">Opening {props.tab} …</p>}>
              <p class="error">{sandbox.error?.message || 'This sandbox could not be read.'}</p>
            </Show>
          }
        >
          {(value) => <TerminalTab sandbox={value()} fixedTab={props.tab} />}
        </Show>
      </div>
    </div>
  );
}
