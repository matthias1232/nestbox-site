// The nodes: the machines sandboxes and models run on.
//
// The screen answers "where does this run" and "why was that model refused" for
// one machine at a time: the last measurement, what the model installs on it
// reserve, and what is left. The numbers are the server's; a node nobody has
// measured says so instead of drawing zeros as capacity.
//
// Two columns are about reachability rather than capacity: "Tunnel" is the state
// of the node's WireGuard tunnel as the hub sees it (refreshed every ten seconds
// and whenever the hub reports a change), and "Egress" is the choice of where the
// node's traffic leaves. Both are admin work, so the page keeps the tunnel list
// small and cheap.
//
// The page itself is markup. State and calls live in `useNodes.js`; the tunnel
// and lock state is small enough to stay here, next to the two columns it feeds.

import { For, Show, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import { useUi } from '../../state/ui.jsx';
import Empty from '../../components/Empty.jsx';
import * as nodesConnect from '../../api/nodesConnect.js';
import { stateStream } from '../../api/telemetry.js';
import BudgetBars from './BudgetBars.jsx';
import NodeDrawer from './NodeDrawer.jsx';
import DeleteNodeDialog from './DeleteNodeDialog.jsx';
import NodeWizard from './NodeWizard.jsx';
import TunnelBadge from './TunnelBadge.jsx';
import { kindLabel, statusTone } from './nodeFormat.js';
import { useNodes } from './useNodes.js';
import '../../styles/nodes.css';

// How often the tunnel list is asked again. The hub writes a status file every
// few seconds, so a quarter of a minute would already be stale-looking; ten
// seconds is one small query and keeps the column honest.
const TUNNEL_REFRESH_MS = 10000;

const LOCK_TEXT = { strict: 'Strict', exit_local: 'Exit locally' };

export default function NodesPage(props) {
  const ui = useUi();
  const picked = (id) => ui?.selection.kind() === 'node' && ui.selection.ids().has(id);
  onCleanup(() => { if (ui?.selection.kind() === 'node') ui.selection.clear(); });
  const state = useNodes();
  const [openId, setOpenId] = createSignal(null);
  const [tunnels, setTunnels] = createSignal({});
  const [tunnelError, setTunnelError] = createSignal(null);
  const [locks, setLocks] = createSignal({});
  const [lockSaving, setLockSaving] = createSignal(null);
  const [lockError, setLockError] = createSignal(null);
  const [lockAsk, setLockAsk] = createSignal(null);
  const [adding, setAdding] = createSignal(false);
  const [deleting, setDeleting] = createSignal(null);

  // One tunnel request at a time: a slow answer must not stack up behind the
  // ten-second poll.
  let tunnelBusy = false;

  const measured = createMemo(() => state.items().filter((node) => node.hardware_known).length);
  const sandboxTotal = createMemo(() =>
    state.items().reduce((sum, node) => sum + (node.budget?.sandboxes || 0), 0));

  /** The tunnel of every node, keyed by node id; one request for the whole table. */
  async function loadTunnels() {
    if (tunnelBusy) return;
    tunnelBusy = true;
    try {
      const list = await nodesConnect.tunnels();
      const byNode = {};
      for (const tunnel of list || []) byNode[tunnel.node_id] = tunnel;
      setTunnels(byNode);
      setTunnelError(null);
    } catch (failure) {
      // A console that cannot read tunnel states still shows its nodes; the page
      // says the column is unknown instead of failing as a whole.
      setTunnelError(failure.message);
    } finally {
      tunnelBusy = false;
    }
  }

  /** Write one node's lock and remember it, so the select shows the server's answer. */
  async function applyLock(node, lock) {
    setLockSaving(node.id);
    setLockError(null);
    try {
      await nodesConnect.setEgressLock(node.id, lock);
      setLocks((current) => ({ ...current, [node.id]: lock }));
    } catch (failure) {
      setLockError(`${node.display_name} (${node.name}): ${failure.message}`);
    } finally {
      setLockSaving(null);
    }
  }

  /**
   * Take a lock picked in the table.
   *
   * `strict` is confirmed first because it moves all of the node's traffic; the
   * control is put back to what the page knows until the server confirms, so a
   * refused change can never look applied.
   */
  function pickLock(node, lock, select) {
    select.value = locks()[node.id] || '';
    if (!lock) return;
    if (lock === 'strict') {
      setLockAsk({ node, lock });
      return;
    }
    applyLock(node, lock);
  }

  onMount(() => {
    // The one allowed location read: `?add=1` (from the home page and the palette) opens the wizard.
    if (new URLSearchParams(window.location.search).get('add') === '1') setAdding(true);
    state.load();
    loadTunnels();
    const timer = setInterval(loadTunnels, TUNNEL_REFRESH_MS);
    // A tunnel that just changed state is exactly the case the next poll would
    // report too late, so the live event refreshes the column at once.
    const stop = stateStream({
      onEvent: (event) => {
        if (event.type === 'node_tunnel_changed') loadTunnels();
      },
      onError: () => {},
    });
    onCleanup(() => {
      clearInterval(timer);
      stop();
    });
  });

  return (
    <section class="page">
      <header class="page-head">
        <div>
          <h1>Nodes</h1>
          <p class="muted">
            {state.items().length} node{state.items().length === 1 ? '' : 's'}
            {' · '}{measured()} measured
            {' · '}{sandboxTotal()} sandbox{sandboxTotal() === 1 ? '' : 'es'} placed
          </p>
        </div>
        <div class="controls">
          <Show when={props.role === 'admin'}>
            <button class="primary" onClick={() => setAdding(true)}>Add node</button>
          </Show>
          <button class="secondary" onClick={() => state.load()} disabled={state.loading()}>
            {state.loading() ? 'Refreshing…' : 'Refresh'}
          </button>
        </div>
      </header>

      <Show when={state.error()}><p class="error">{state.error()}</p></Show>
      <Show when={tunnelError()}>
        <p class="muted small">Tunnel states unavailable: {tunnelError()}</p>
      </Show>
      <Show when={lockError()}><p class="error">{lockError()}</p></Show>

      <Show when={state.items().length > 0}>
        <div class="table-scroll">
          <table class="node-table">
            <thead>
              <tr>
                <th class="narrow" />
                <th>Node</th>
                <th class="narrow">Kind</th>
                <th class="narrow">Status</th>
                <th class="narrow">Tunnel</th>
                <th class="narrow">Egress</th>
                <th class="narrow">Container GPU</th>
                <th class="narrow">Sandboxes</th>
                <th>Budget</th>
                <Show when={props.role === 'admin'}><th class="narrow">Actions</th></Show>
              </tr>
            </thead>
            <tbody>
              <For each={state.items()}>
                {(node) => (
                  <tr class={openId() === node.id ? 'selected' : ''}>
                    <td class="narrow">
                      <input type="checkbox" aria-label={`Select ${node.display_name}`} checked={picked(node.id)} onChange={() => ui?.selection.toggle('node', node.id)} />
                    </td>
                    <td>
                      <button class="link" onClick={() => setOpenId(node.id)}>
                        {node.display_name}
                      </button>
                      <div class="muted small mono">{node.name}</div>
                      <Show when={node.description}>
                        <div class="muted small node-desc">{node.description}</div>
                      </Show>
                    </td>
                    <td class="narrow small">
                      {kindLabel(node.kind)}
                      <Show when={node.kind === 'kubernetes'}>
                        <span
                          class="badge warn tiny-badge"
                          title="Declared in the data model; no adapter runs sandboxes on it yet."
                        >
                          <i />no adapter
                        </span>
                      </Show>
                      <Show when={node.is_local}>
                        <span class="badge muted tiny-badge" title="This is the host the control plane runs on.">
                          <i />this host
                        </span>
                      </Show>
                    </td>
                    <td class="narrow">
                      <span class={`badge ${statusTone(node.status)}`}><i />{node.status}</span>
                    </td>
                    <td class="narrow small">
                      <Show
                        when={!node.is_local}
                        fallback={
                          <span class="muted" title="The local host is this gateway; it has no tunnel to it.">
                            local
                          </span>
                        }
                      >
                        <Show
                          when={tunnels()[node.id]}
                          fallback={<span class="muted" title="The hub has no peer for this node yet.">no tunnel</span>}
                        >
                          <TunnelBadge
                            state={tunnels()[node.id].state}
                            age={tunnels()[node.id].handshake_age_seconds}
                            title={tunnels()[node.id].endpoint
                              ? `Endpoint ${tunnels()[node.id].endpoint}`
                              : 'The hub has not seen this peer yet.'}
                          />
                        </Show>
                      </Show>
                    </td>
                    <td class="narrow small">
                      <Show
                        when={!node.is_local}
                        fallback={
                          <span class="muted" title="The local host is this gateway; its own traffic does not take a tunnel.">
                            local
                          </span>
                        }
                      >
                        <select
                          value={locks()[node.id] || ''}
                          disabled={lockSaving() === node.id}
                          title="Where this node's traffic leaves. The node applies a change within five minutes."
                          onChange={(event) => pickLock(node, event.currentTarget.value, event.currentTarget)}
                        >
                          <option value="" disabled>
                            {locks()[node.id] ? LOCK_TEXT[locks()[node.id]] : 'not read'}
                          </option>
                          <option value="strict">Strict</option>
                          <option value="exit_local">Exit locally</option>
                        </select>
                      </Show>
                    </td>
                    <td class="narrow small">
                      <Show
                        when={node.container_gpu}
                        fallback={
                          <Show
                            when={node.hardware_known}
                            fallback={<span class="muted" title="Nobody has probed this node, so the runner has not answered this either.">not measured yet</span>}
                          >
                            <span class="muted" title="Measured by the runner: a container on this node cannot be given a GPU.">
                              no
                            </span>
                          </Show>
                        }
                      >
                        <span class="badge ok" title="Measured by the runner: a container here can be given a GPU.">
                          <i />yes
                        </span>
                      </Show>
                    </td>
                    <td class="narrow mono">
                      <button
                        class="link"
                        title="Show the sandboxes on this node"
                        onClick={() => setOpenId(node.id)}
                      >
                        {node.budget?.sandboxes ?? 0}
                      </button>
                    </td>
                    <td class="node-budget">
                      <BudgetBars
                        budget={node.budget}
                        hardwareKnown={node.hardware_known}
                        detectedAt={node.detected_at}
                        compact
                      />
                    </td>
                    <Show when={props.role === 'admin'}>
                      <td class="narrow">
                        <Show when={!node.is_local}>
                          <button class="danger tiny" onClick={() => setDeleting(node)}>Delete…</button>
                        </Show>
                      </td>
                    </Show>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>

      <Show when={state.loaded() && state.items().length === 0}>
        <Empty
          title="No node yet"
          hint="A data center runs sandboxes on its own nodes. Add a machine below: you get one line to paste on it, and it shows up here within a minute."
        />
      </Show>

      <Show when={lockAsk()}>
        <div class="warn-note">
          <p>
            <strong>{lockAsk().node.display_name}</strong>: the node will send all traffic through
            this gateway. Running sandboxes keep working; downloads become slower if this gateway's
            line is slow.
          </p>
          <div class="actions">
            <button
              class="primary"
              onClick={() => {
                const ask = lockAsk();
                setLockAsk(null);
                if (ask) applyLock(ask.node, ask.lock);
              }}
            >
              Send everything through this gateway
            </button>
            <button class="secondary" onClick={() => setLockAsk(null)}>Cancel</button>
          </div>
        </div>
      </Show>

      <Show when={props.role === 'admin'}>
        <Show when={adding()}>
          <NodeWizard onClose={() => setAdding(false)} onDone={() => state.load()} />
        </Show>
        <Show when={deleting()}>
          <DeleteNodeDialog node={deleting()} nodes={state.items()} onClose={() => setDeleting(null)}
                            onDeleted={() => { setDeleting(null); state.load(); }} />
        </Show>
      </Show>

      <p class="muted small phase-note">
        A budget is the node's last measurement minus what the models installed on it
        reserve; it is not a live reading of free memory. A node that has never been
        probed has no budget, and this page says so instead of showing zeros.
      </p>

      <NodeDrawer
        nodeId={openId()}
        onClose={() => setOpenId(null)}
        onChanged={() => state.load()}
        onOpenSandbox={props.onOpenSandbox}
      />
    </section>
  );
}
