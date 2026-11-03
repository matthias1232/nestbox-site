// One sandbox in the Workbox: header with its lifecycle, the three cards, the
// two live panels and the shared sessions.
//
// This file decides placement and nothing else. Every card owns its own reads
// and its own changes, so a broken card never blanks the page, and the page's
// one hook only supplies what several children would otherwise ask for twice.

import { Show, createSignal } from 'solid-js';
import ConfirmDialog from '../../components/ConfirmDialog.jsx';
import PresenceDots from '../../components/PresenceDots.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import * as deck from '../../api/deck.js';
import * as sandboxApi from '../../api/sandboxes.js';
import PageHeader from '../shell/PageHeader.jsx';
import { navigate, paths } from '../../routing.js';
import { useSession } from '../../state/session.jsx';
import { useUi } from '../../state/ui.jsx';
import AgentsCard from './AgentsCard.jsx';
import AllowAllBanner from './AllowAllBanner.jsx';
import ControlStrip from './ControlStrip.jsx';
import LiveLogCard from './LiveLogCard.jsx';
import RulesFromLogCard from './RulesFromLogCard.jsx';
import LiveChatsPanel from './LiveChatsPanel.jsx';
import ModelsCard from './ModelsCard.jsx';
import MoveDialog from './MoveDialog.jsx';
import NetworkCard from './NetworkCard.jsx';
import LiveEventsPanel from './LiveEventsPanel.jsx';
import RequestStreamCard from './RequestStreamCard.jsx';
import WorkboxNotice from './WorkboxNotice.jsx';
import PrivacyCard from './PrivacyCard.jsx';
import SessionsPanel from './SessionsPanel.jsx';
import TopologyPanel from './TopologyPanel.jsx';
import { useWorkbox } from './useWorkbox.js';
import '../../styles/workbox.css';

// Opening the Admin Center is an operator's tool; a member sees the workbox only.
const ADMIN_ROLES = ['operator', 'admin'];

// The statuses in which a container exists and can be stopped.
const LIVE_STATUSES = ['running', 'starting', 'provisioning'];
// The statuses in which starting it again is the only lifecycle action offered.
const RESTING_STATUSES = ['stopped', 'failed'];

/**
 * The page, keyed by sandbox id.
 *
 * Keyed on purpose: following a link from one box to another starts the page
 * from scratch instead of showing the previous sandbox's agents and windows for
 * the moment until the first answer arrives.
 */
export default function WorkboxPage(props) {
  return (
    <Show when={props.sandboxId} keyed fallback={<p class="muted">No sandbox chosen.</p>}>
      {(id) => <Workbox id={id} />}
    </Show>
  );
}

// One sandbox, with the state and the actions its header needs.
function Workbox(props) {
  const session = useSession();
  const ui = useUi();
  const state = useWorkbox(() => props.id);

  const [busy, setBusy] = createSignal(false);
  const [actionError, setActionError] = createSignal(null);
  const [recreating, setRecreating] = createSignal(false);
  const [moving, setMoving] = createSignal(false);
  const [consoleMenu, setConsoleMenu] = createSignal(false);

  // Remember the choice and bring the sessions panel into view.
  function openConsole(mode) {
    ui.console.setMode(props.id, mode);
    setConsoleMenu(false);
    document.getElementById('wb-sessions')?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  const sandbox = () => state.sandbox();
  const summary = () => state.control() || {};
  const name = () => sandbox()?.name || summary().name || props.id;
  const status = () => sandbox()?.status || summary().status || 'unknown';
  const nodeName = () => summary().node?.name || sandbox()?.node_name || 'no node yet';

  // Who is connected: an SSH session carries no user name, and saying "SSH" is
  // better than an empty dot that looks like nobody.
  const people = () =>
    (Array.isArray(summary().presence) ? summary().presence : []).map((entry) => ({
      name: entry.user_name || 'SSH',
      kind: entry.kind,
      view_only: entry.view_only,
    }));

  const running = () => LIVE_STATUSES.includes(status());
  const resting = () => RESTING_STATUSES.includes(status());
  const canAdmin = () => ADMIN_ROLES.includes(session.identity()?.role);

  // Every action ends the same way: reload the whole picture, or say why not.
  async function run(work) {
    setBusy(true);
    setActionError(null);
    try {
      await work();
      await state.refresh();
    } catch (failure) {
      setActionError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  function confirmRecreate() {
    setRecreating(false);
    run(() => deck.recreate(props.id));
  }

  return (
    <section class="page wb-page">
      <Show when={!sandbox() && !summary().sandbox_id && !state.error()}>
        <p class="muted">Loading…</p>
      </Show>

      <Show when={state.error()}>
        <div class="wb-error">
          <p class="error">{state.error()}</p>
          <button class="secondary tiny" onClick={() => state.refresh()}>Retry</button>
        </div>
      </Show>

      <Show when={sandbox() || summary().sandbox_id}>
        <PageHeader
          title={name()}
          status={<StatusBadge status={status()} />}
          subtitle={nodeName()}
          actions={
            <div class="wb-actions">
              <PresenceDots people={people()} />
              <Show when={resting()}>
                <button class="primary" disabled={busy()} onClick={() => run(() => sandboxApi.act(props.id, 'start'))}>
                  Start
                </button>
              </Show>
              <Show when={running()}>
                <button class="secondary" disabled={busy()} onClick={() => run(() => sandboxApi.act(props.id, 'stop'))}>
                  Stop
                </button>
                <button class="secondary" disabled={busy()} onClick={() => run(() => sandboxApi.act(props.id, 'restart'))}>
                  Restart
                </button>
              </Show>
              <button class="secondary" disabled={busy()} onClick={() => setRecreating(true)}>Recreate</button>
              <button class="secondary" disabled={busy()} onClick={() => setMoving(true)}>Move…</button>
              <Show when={canAdmin()}>
                <button class="ghost" onClick={() => navigate(paths.sandbox(props.id))}>Admin view</button>
              </Show>
              <div class="wb-split">
                <button class="primary" onClick={() => openConsole(ui.console.mode(props.id) || 'shell')}>
                  Console
                </button>
                <button
                  class="primary wb-split-more"
                  aria-label="Choose the console"
                  aria-expanded={consoleMenu()}
                  onClick={() => setConsoleMenu(!consoleMenu())}
                >
                  ▾
                </button>
                <Show when={consoleMenu()}>
                  <div class="wb-split-menu" role="menu">
                    <button role="menuitem" class="ghost" onClick={() => openConsole('shell')}>Shell</button>
                    <button role="menuitem" class="ghost" onClick={() => openConsole('desktop')}>Desktop</button>
                  </div>
                </Show>
              </div>
            </div>
          }
        />

        <Show when={actionError()}>
          <p class="error">{actionError()}</p>
        </Show>

        <AllowAllBanner control={summary()} sandboxId={props.id} onChanged={state.refresh} />

        <ControlStrip sandboxId={props.id} control={summary()} onChanged={state.refresh} />

        <WorkboxNotice sandboxId={props.id} />

        <div class="wb-cards">
          <AgentsCard sandboxId={props.id} control={summary()} onChanged={state.refresh} />
          <ModelsCard sandboxId={props.id} control={summary()} />
          <NetworkCard sandboxId={props.id} control={summary()} onChanged={state.refresh} />
          <PrivacyCard sandboxId={props.id} />
          <RequestStreamCard sandboxId={props.id} />
        </div>

        <div class="wb-main">
          <LiveLogCard sandboxId={props.id} onChanged={state.refresh} />
          <RulesFromLogCard sandboxId={props.id} control={summary()} />
        </div>

        <Show when={sandbox()}>
          <div class="wb-main">
            <LiveChatsPanel sandbox={sandbox()} />
            <TopologyPanel sandbox={sandbox()} />
            <LiveEventsPanel sandbox={sandbox()} />
          </div>

          <div id="wb-sessions">
            <SessionsPanel sandbox={sandbox()} control={summary()} />
          </div>
        </Show>

        <MoveDialog
          open={moving()}
          sandboxId={props.id}
          sandboxName={name()}
          currentNode={nodeName()}
          onClose={() => setMoving(false)}
          onMoved={() => {
            setMoving(false);
            state.refresh();
          }}
        />

        <ConfirmDialog
          open={recreating()}
          title={`Recreate ${name()}?`}
          onClose={() => setRecreating(false)}
          actions={[
            { label: 'Recreate', tone: 'danger', onClick: confirmRecreate },
            { label: 'Cancel', tone: 'secondary', onClick: () => setRecreating(false) },
          ]}
        >
          <p>
            The container is made again from its image. Persistent folders and logins stay;
            anything else inside the container is lost.
          </p>
        </ConfirmDialog>
      </Show>
    </section>
  );
}
