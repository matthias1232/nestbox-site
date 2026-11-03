// The agents card: one row per agent with its sign-in mode, its login and its sessions.
//
// Switching the mode restarts the agent, and a restart ends whatever the agent is
// doing right now. So the switch asks first when something runs: the dialog offers
// to wait for a busy answer, to restart everything at once, or to leave the running
// sessions alone. Nothing here decides which sessions are busy; that is the
// runtime's answer, read out through `agents.js`.

import { For, Show, createSignal } from 'solid-js';
import ConfirmDialog from '../../components/ConfirmDialog.jsx';
import Segmented from '../../components/Segmented.jsx';
import * as agentAuth from '../../api/agentAuth.js';
import { agentLabel, busyMessage, loginHint, loginLine, MODE_OPTIONS, modeLabel, sessionSummary } from './agents.js';

export default function AgentsCard(props) {
  // The dialog that is open: { state, mode, restartOnly } or null.
  const [pending, setPending] = createSignal(null);
  // The agent whose call is running right now.
  const [working, setWorking] = createSignal(null);
  // The last failure: { agent, message }.
  const [failure, setFailure] = createSignal(null);

  const agents = () => props.control?.agents || [];

  // The id may arrive as a string or as a signal accessor; both are allowed.
  function sandboxId() {
    const value = props.sandboxId;
    return typeof value === 'function' ? value() : value;
  }

  function isWorking(state) {
    return working() === state.agent;
  }

  async function run(state, action) {
    setWorking(state.agent);
    setFailure(null);
    try {
      await action();
      props.onChanged?.();
    } catch (error) {
      setFailure({ agent: state.agent, message: error?.message || 'The switch failed.' });
    } finally {
      setWorking(null);
    }
  }

  function choose(state, mode) {
    if (mode === state.mode) return;
    if (state.busy || (state.sessions || []).length > 0) {
      setFailure(null);
      setPending({ state, mode, restartOnly: false });
      return;
    }
    run(state, () => agentAuth.set(sandboxId(), state.agent, mode, 'idle'));
  }

  function chooseRestart(state) {
    setFailure(null);
    setPending({ state, mode: state.mode, restartOnly: true });
  }

  function close() {
    setPending(null);
  }

  function confirm(when) {
    const item = pending();
    setPending(null);
    if (!item) return;
    if (item.restartOnly) {
      run(item.state, () => agentAuth.restart(sandboxId(), item.state.agent, when));
      return;
    }
    run(item.state, () => agentAuth.set(sandboxId(), item.state.agent, item.mode, when));
  }

  function dialogTitle() {
    const item = pending();
    if (!item) return '';
    if (item.restartOnly) return `Restart ${agentLabel(item.state.agent)}?`;
    return `Switch ${agentLabel(item.state.agent)} to ${modeLabel(item.mode)}?`;
  }

  const actions = [
    { label: 'Restart idle now, busy ones when done', tone: 'primary', onClick: () => confirm('idle') },
    { label: 'Restart all now', tone: 'danger', onClick: () => confirm('now') },
    { label: 'Only for new sessions', tone: 'secondary', onClick: () => confirm('later') },
    { label: 'Cancel', tone: 'secondary', onClick: () => close() },
  ];

  return (
    <div class="wb-card">
      <h3>Agents · model access</h3>

      <Show
        when={agents().length > 0}
        fallback={<p class="muted">This sandbox has no agent whose sign-in can be switched.</p>}
      >
        <For each={agents()}>
          {(state) => (
            <div class="agent-row">
              <div>
                <strong>{agentLabel(state.agent)}</strong>
                <span class="muted small"> {loginLine(state)}</span>
                <Show when={state.busy}>
                  <span class="badge warn"><i />working</span>
                </Show>
                <Show when={state.pending_restart}>
                  <span class="badge info"><i />restart waiting</span>
                </Show>
              </div>

              <div class="actions">
                <Segmented
                  options={MODE_OPTIONS}
                  value={state.mode}
                  onChange={(mode) => choose(state, mode)}
                  disabled={isWorking(state)}
                />
                <button class="tiny" disabled={isWorking(state)} onClick={() => chooseRestart(state)}>
                  Restart
                </button>
                <Show when={isWorking(state)}>
                  <span class="muted small">Switching…</span>
                </Show>
              </div>

              <Show when={sessionSummary(state)}>
                <p class="muted small">{sessionSummary(state)}</p>
              </Show>

              <Show when={loginHint(state)}>
                <p class="muted small">{loginHint(state)}</p>
              </Show>

              <Show when={failure()?.agent === state.agent}>
                <p class="error">{failure().message}</p>
              </Show>
            </div>
          )}
        </For>
      </Show>

      <Show when={pending()}>
        {(item) => (
          <ConfirmDialog open={true} title={dialogTitle()} onClose={close} actions={actions}>
            <Show when={busyMessage(item().state)}>
              <p>{busyMessage(item().state)}</p>
            </Show>
            <p class="muted small">Your own login stays saved; you will not have to sign in again.</p>
          </ConfirmDialog>
        )}
      </Show>
    </div>
  );
}
