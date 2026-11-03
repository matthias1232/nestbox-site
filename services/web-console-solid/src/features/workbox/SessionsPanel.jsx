// The shared sessions of one sandbox: every terminal tab, and the desktop.
//
// Two facts drive this panel. A tab is one tmux session, so several people can
// be in it at once, and the panel says who — a shared shell that nobody can see
// is a shared shell two people type into at the same time. The desktop is the
// same sandbox from the other side: what the agents see in Chrome.
//
// The panel owns no data: the sandbox comes in as a prop and who is connected
// comes from the control summary (`control.presence`), so the Workbox has one
// source for "who is here" instead of one poll per panel.

import { Show } from 'solid-js';
import ConsoleFrame from '../../components/ConsoleFrame.jsx';
import PresenceDots from '../../components/PresenceDots.jsx';
import '../../styles/workbox-sessions.css';

// Who is connected, in the shape `components/PresenceDots.jsx` reads. One dot
// per open session, so two terminals of the same person are two dots.
function people(presence) {
  return (presence || []).map((entry) => ({
    name: entry.user_name || 'someone',
    kind: entry.kind,
    view_only: entry.view_only === true,
  }));
}

// The names behind those dots, each once, in the order they arrived.
function names(presence) {
  const seen = [];
  for (const entry of presence || []) {
    const name = entry.user_name || 'someone';
    if (!seen.includes(name)) seen.push(name);
  }
  return seen;
}

// How many different tabs people are sitting in.
function tabCount(presence) {
  const ids = new Set();
  for (const entry of presence || []) {
    if (entry.tab_id) ids.add(entry.tab_id);
  }
  return ids.size;
}

// "Alice, Bo in 2 tabs" — or that nobody is here, which is also information.
function presenceLine(presence) {
  const list = names(presence);
  const count = tabCount(presence);
  if (!list.length) return 'Nobody is connected right now.';
  const who = list.join(', ');
  if (!count) return `${who} in this sandbox`;
  return `${who} in ${count} ${count === 1 ? 'tab' : 'tabs'}`;
}

// The sessions of one sandbox: the shared terminals, and the desktop.
export default function SessionsPanel(props) {
  const presence = () => props.control?.presence || [];

  return (
    <section class="ws-sessions">
      <div class="ws-presence">
        <strong>Connected now</strong>
        <Show when={presence().length > 0}>
          <PresenceDots people={people(presence())} />
        </Show>
        <span class="small">{presenceLine(presence())}</span>
      </div>
      <ConsoleFrame sandbox={props.sandbox} role={props.role} />
    </section>
  );
}
