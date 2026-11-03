// Gateways, supervisors, and the cluster graph.

import { For, Show, createMemo, createSignal, onMount } from 'solid-js';
import StatusBadge from '../../components/StatusBadge.jsx';
import TopologyCanvas from '../../components/TopologyCanvas.jsx';
import { fromCluster } from '../../components/topologyLayout.js';
import { navigate, paths } from '../../routing.js';
import Tabs from '../../components/Tabs.jsx';
import * as cluster from '../../api/cluster.js';
import * as policies from '../../api/policies.js';

const TABS = [
  { id: 'graph', label: 'Topology' },
  { id: 'gateways', label: 'Gateways' },
  { id: 'supervisors', label: 'Supervisors' },
];

export default function ClusterPage() {
  const [tab, setTab] = createSignal('graph');
  const [gateways, setGateways] = createSignal([]);
  const [supervisors, setSupervisors] = createSignal([]);
  const [topology, setTopology] = createSignal({ nodes: [], edges: [] });
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);
  // The traffic window drawn on the graph. Without a window the graph is
  // placement only, and a blocked destination is invisible here.
  const [live, setLive] = createSignal('15m');
  // The card an operator selected on the canvas.
  const [selectedId, setSelectedId] = createSignal(null);
  const [busy, setBusy] = createSignal(false);

  onMount(load);

  async function load() {
    setError(null);
    try {
      const [gatewayList, supervisorList, graph] = await Promise.all([
        cluster.gateways(),
        cluster.supervisors(),
        cluster.topology(live()),
      ]);
      setGateways(gatewayList.items || []);
      setSupervisors(supervisorList.items || []);
      setTopology({ nodes: graph.nodes || [], edges: graph.edges || [] });
    } catch (failure) {
      setError(failure.message);
    }
  }

  /// Allow one destination for a bounded time, on its own firewall.
  ///
  /// The channel comes from the destination's protocol: a name lookup and a
  /// connection are different firewalls, and opening the wrong one would be a
  /// wider permission than the operator asked for.
  async function allowDestination(node, minutes) {
    const channel = node.protocol === 'dns' ? 'dns' : node.protocol === 'mcp' ? 'mcp' : 'proxy';
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await policies.openWindow(
        null,
        minutes,
        `Allow ${node.name} from the topology`,
        'bypass',
        channel,
      );
      setNotice(`${node.name} is open for ${minutes} minutes on ${channel}.`);
      await load();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  // The canvas data: the cluster answer in the shape of contract C15.
  const drawing = createMemo(() => fromCluster(topology().nodes, topology().edges));

  /// What a canvas action does on this page: navigate, or open a bounded window.
  function onCanvasAction(id, action) {
    const item = drawing().nodes.find((entry) => entry.id === id);
    if (!item) return;
    if (action.startsWith('allow:')) {
      allowDestination(item.source || { name: item.label }, Number(action.slice(6)));
    } else if (action === 'policies') {
      navigate(paths.menu('policies'));
    } else if (item.kind === 'sandbox') {
      navigate(paths.sandbox(id, action === 'logs' ? 'logs' : undefined));
    } else if (action === 'logs') {
      navigate(paths.menu('logs'));
    }
  }

  /// The checks the supervisor measured, in the order it reported them.
  function checksOf(supervisor) {
    return supervisor?.reported?.checks || [];
  }

  /// Queue one whitelisted action and say what happened.
  async function act(supervisor, kind) {
    if (!supervisor.sandbox_id) {
      setNotice(`${supervisor.name} is not bound to a sandbox.`);
      return;
    }
    setError(null);
    setNotice(null);
    try {
      const outcome = await cluster.supervisorAction(supervisor.sandbox_id, kind);
      setNotice(`${kind.replaceAll('_', ' ')} queued for ${supervisor.name} (${outcome.command_id.slice(0, 8)})`);
      await load();
    } catch (failure) {
      setError(failure.message);
    }
  }

  async function sync(gateway) {
    setNotice(null);
    setError(null);
    try {
      const outcome = await cluster.sync(gateway.id);
      setNotice(`Sync: ${outcome.outcome.replaceAll('_', ' ')} (applied v${outcome.applied_version})`);
      await load();
    } catch (failure) {
      setError(failure.message);
    }
  }

  return (
    <section class="page">
      <header class="page-head">
        <div>
          <h1>Cluster</h1>
          <p class="muted">
            {gateways().length} gateways · {supervisors().length} supervisors
          </p>
        </div>
        <div class="controls">
          <button class="secondary" onClick={load}>Refresh</button>
        </div>
      </header>

      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={notice()}><p class="notice">{notice()}</p></Show>

      <Tabs tabs={TABS} active={tab()} onChange={setTab} />

      <Show when={tab() === 'graph'}>
        <div class="controls">
          <button class="secondary tiny" onClick={load}>Refresh</button>
        </div>

        <TopologyCanvas
          label="Cluster topology"
          nodes={drawing().nodes}
          edges={drawing().edges}
          layers={['supervisor', 'gateway', 'sandbox', 'egress', 'destination']}
          selectedId={selectedId()}
          onSelect={setSelectedId}
          onAction={onCanvasAction}
          trafficWindows={['', '5m', '15m', '1h']}
          trafficWindow={live()}
          onTrafficWindow={(value) => {
            setLive(value);
            load();
          }}
          busy={busy()}
          height={560}
        />

        <p class="muted small">
          Solid lines are placement: the cluster runs the gateway, the gateway
          hosts the sandbox. Dashed lines are traffic: a sandbox's egress goes
          back through its gateway, where the policy decides. A sandbox never
          reaches the network directly. A red destination had requests refused in
          the window; click it to allow it for a while.
        </p>
      </Show>

      <Show when={tab() === 'gateways'}>
        <Show
          when={gateways().length > 0}
          fallback={<p class="muted">No gateway registered yet.</p>}
        >
          <div class="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Status</th>
                  <th>Version</th>
                  <th class="narrow">Config</th>
                  <th class="narrow">Capacity</th>
                  <th>Last seen</th>
                  <th>Actions</th>
                </tr>
              </thead>
              <tbody>
                <For each={gateways()}>
                  {(gateway) => {
                    const drifted = gateway.settings_applied_version < gateway.settings_version;
                    return (
                      <tr>
                        <td class="mono">{gateway.name}</td>
                        <td><StatusBadge status={gateway.status} /></td>
                        <td class="mono small">{gateway.version || '—'}</td>
                        <td class="narrow">
                          <Show when={drifted}
                                fallback={<span class="badge ok"><i />in sync</span>}>
                            <span class="badge warn" title={`desired v${gateway.settings_version}, applied v${gateway.settings_applied_version}`}>
                              <i />drift
                            </span>
                          </Show>
                        </td>
                        <td class="narrow mono small">
                          {gateway.used}/{gateway.capacity}
                        </td>
                        <td class="mono small">{gateway.last_seen || '—'}</td>
                        <td class="actions">
                          <button class="tiny" disabled={!drifted} onClick={() => sync(gateway)}>
                            Sync now
                          </button>
                        </td>
                      </tr>
                    );
                  }}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </Show>

      <Show when={tab() === 'supervisors'}>
        <Show
          when={supervisors().length > 0}
          fallback={<p class="muted">No supervisor reported yet.</p>}
        >
          <div class="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>Name</th><th>Status</th><th class="narrow">Config dirtiness</th>
                  <th>Last policy apply</th><th>Last seen</th>
                  <th>Checks</th><th>Actions</th>
                </tr>
              </thead>
              <tbody>
                <For each={supervisors()}>
                  {(supervisor) => (
                    <tr>
                      <td class="mono">{supervisor.name}</td>
                      <td><StatusBadge status={supervisor.status} /></td>
                      <td class="narrow mono">
                        {supervisor.config_dirtiness > 0
                          ? <span class="badge warn"><i />{supervisor.config_dirtiness}</span>
                          : <span class="badge ok"><i />0</span>}
                      </td>
                      <td class="mono small">{supervisor.last_policy_applied_at || '—'}</td>
                      <td class="small">
                        <For each={checksOf(supervisor)}>
                          {(check) => (
                            <div class={`check ${check.state}`} title={check.detail}>
                              <i class={`dot ${check.state === 'ok' ? 'ok' : check.state === 'warn' ? 'warn' : 'bad'}`} />
                              {check.key}
                            </div>
                          )}
                        </For>
                        <Show when={checksOf(supervisor).length === 0}>
                          <span class="muted">no report yet</span>
                        </Show>
                      </td>
                      <td class="controls">
                        <button class="tiny" onClick={() => act(supervisor, 'run_check')}>Check</button>
                        <button class="tiny" onClick={() => act(supervisor, 'restart_agent')}>Restart</button>
                      </td>
                      <td class="mono small">{supervisor.last_seen || '—'}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </Show>
    </section>
  );
}
