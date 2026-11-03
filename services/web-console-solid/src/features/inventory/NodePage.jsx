// A node: breadcrumb, title with status, sticky action bar and tabs (contract C1, C8).
import { For, Show, createMemo, createSignal } from 'solid-js';
import Breadcrumb from '../shell/Breadcrumb.jsx';
import PageHeader from '../shell/PageHeader.jsx';
import { treePathFor } from '../shell/treeFilter.js';
import { useInventory } from '../shell/useInventory.js';
import Tabs from '../../components/Tabs.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import Segmented from '../../components/Segmented.jsx';
import ConfirmDialog from '../../components/ConfirmDialog.jsx';
import LogViewer from '../../components/LogViewer.jsx';
import TrafficTable from '../../components/TrafficTable.jsx';
import { usePolling } from '../../components/Poll.js';
import { useSession } from '../../state/session.jsx';
import { useUi } from '../../state/ui.jsx';
import { navigate, paths } from '../../routing.js';
import * as nodesApi from '../../api/nodes.js';
import * as nodeOps from '../../api/nodeOps.js';
import * as nodesConnect from '../../api/nodesConnect.js';
import * as imagesApi from '../../api/images.js';
import * as sandboxesApi from '../../api/sandboxes.js';
import * as trafficApi from '../../api/traffic.js';
import NodeSummaryTab from './NodeSummaryTab.jsx';
import ScopedSandboxesTab from './ScopedSandboxesTab.jsx';
import ScopedModelsTab from './ScopedModelsTab.jsx';
import { useNodeSummary, useTabState } from './useNodeSummary.js';
import { nodesBelow, gib } from './inventoryTabs.js';
import '../../styles/inventory.css';

const STATUS_WORD = { online: 'running', degraded: 'degraded', offline: 'offline', unknown: 'unknown' };

// Images of this node, from the images matrix.
function NodeImagesTab(props) {
  const [cells, setCells] = createSignal([]);
  const [error, setError] = createSignal(null);
  usePolling(() => imagesApi.matrix(), {
    everyMs: 15000,
    onData: (matrix) => { setCells((matrix.cells || []).filter((cell) => cell.node_id === props.nodeId)); setError(null); },
    onError: (failure) => setError(failure.message),
  });
  return (
    <div class="inv-tab">
      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={cells().length > 0} fallback={<p class="muted">No image is assigned to this node yet. Assign one on the Images page.</p>}>
        <div class="table-scroll">
          <table>
            <thead><tr><th>Image</th><th>State</th><th class="num">Size</th><th>Note</th></tr></thead>
            <tbody>
              <For each={cells()}>
                {(cell) => (
                  <tr>
                    <td class="mono small">{cell.image}</td>
                    <td><StatusBadge status={cell.state === 'present' ? 'running' : cell.state === 'failed' ? 'failed' : cell.state === 'pulling' ? 'downloading' : 'unknown'} label={cell.state} /></td>
                    <td class="num mono">{gib(cell.size_bytes)}</td>
                    <td class="small muted">{cell.error || ''}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </div>
  );
}

// Recent traffic of this node: the environment list filtered client-side by the node's sandboxes
// (the API has no node filter).
function NodeTrafficTab(props) {
  const [rows, setRows] = createSignal([]);
  const [error, setError] = createSignal(null);
  usePolling(async () => {
    const [page, boxes] = await Promise.all([
      trafficApi.environment({ window: '15m', filter: 'all', limit: 200 }),
      nodesApi.sandboxes(props.nodeId),
    ]);
    const names = {};
    for (const entry of boxes.items || []) {
      const box = entry.sandbox || entry;
      names[box.id] = box.name;
    }
    return (page.items || [])
      .filter((item) => names[item.sandbox_id])
      .map((item) => ({
        id: item.id,
        ts: item.ts,
        sandboxId: item.sandbox_id,
        sandbox: names[item.sandbox_id],
        kind: item.protocol === 'dns' ? 'dns' : item.app_protocol === 'mcp' ? 'mcp' : 'network',
        decision: item.decision,
        host: item.host,
        port: item.port,
        protocol: item.protocol,
        method: item.method,
        path: item.path,
        tool: item.mcp_tool,
        channel: item.channel,
        reason: item.reason_text || item.reason,
        binary: item.binary,
      }));
  }, {
    everyMs: 5000,
    onData: (list) => { setRows(list); setError(null); },
    onError: (failure) => setError(failure.message),
  });
  return (
    <div class="inv-tab">
      <p class="muted small">The last 15 minutes of the sandboxes on this node.</p>
      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={rows().length > 0} fallback={<p class="muted">No traffic in the last 15 minutes.</p>}>
        <TrafficTable events={rows()} />
      </Show>
    </div>
  );
}

// The logs of every sandbox of this node, merged by time and prefixed with the sandbox name.
function NodeLogsTab(props) {
  async function load() {
    const boxes = await nodesApi.sandboxes(props.nodeId);
    const running = (boxes.items || []).map((entry) => entry.sandbox || entry).filter((box) => box.status === 'running');
    const results = await Promise.allSettled(running.map((box) => sandboxesApi.logs(box.id, 80)));
    const lines = [];
    results.forEach((result, index) => {
      if (result.status !== 'fulfilled') return;
      const text = typeof result.value === 'string' ? result.value : result.value?.logs || result.value?.text || '';
      for (const line of String(text).split('\n')) if (line.trim()) lines.push(`[${running[index].name}] ${line}`);
    });
    return lines.sort().join('\n');
  }
  return (
    <div class="inv-tab">
      <LogViewer load={load} source={props.nodeId} />
    </div>
  );
}

// Props: `id` (node id).
export default function NodePage(props) {
  const session = useSession();
  const ui = useUi();
  const inventory = useInventory();
  const summary = useNodeSummary(() => props.id);
  const role = () => session.identity()?.role || 'member';
  const state = useTabState('node', role, () => props.tab, () => paths.node(props.id));
  const [busy, setBusy] = createSignal(null);
  const [dialog, setDialog] = createSignal(null); // 'drain' | 'delete' | 'lock'
  const [target, setTarget] = createSignal('');
  const [lock, setLock] = createSignal(null);

  const isOperator = () => role() === 'operator' || role() === 'admin';
  const node = () => summary.data()?.node || {};
  const name = () => node().name || '';
  const title = () => node().display_name || name() || 'Node';
  const status = () => node().status || 'unknown';
  const currentLock = () => lock() || summary.data()?.tunnel?.egress_lock || null;
  const crumbs = createMemo(() => {
    const trail = treePathFor(inventory.root(), 'node', props.id);
    return trail.length ? trail : [{ label: 'Nodes', path: paths.menu('nodes') }, { label: title() }];
  });
  const siblings = createMemo(() => nodesBelow(inventory.root()).filter((n) => n.id !== props.id && n.status === 'online'));
  // V08: the strip carries the sandbox and model counts the summary already knows.
  const tabs = () => state.tabs().map((tab) => {
    const data = summary.data();
    if (!data) return tab;
    if (tab.id === 'sandboxes') return { ...tab, count: data.sandboxes?.total ?? 0 };
    if (tab.id === 'models') return { ...tab, count: data.installs?.length ?? 0 };
    return tab;
  });
  const toast = (tone, text) => ui?.toast.show({ tone, text, undo: null, ms: 6000 });

  async function run(label, work) {
    setBusy(label);
    try {
      await work();
    } catch (failure) {
      toast('bad', failure.message || `${label} failed.`);
    } finally {
      setBusy(null);
    }
  }
  const probe = () => run('probe', async () => {
    await nodesApi.probe(props.id);
    toast('ok', `Measured ${title()}.`);
    summary.refresh();
  });
  const drain = (stopOnly) => run('drain', async () => {
    const answer = await nodeOps.drain(props.id, { to_node: stopOnly ? null : target(), stop_only: stopOnly });
    setDialog(null);
    toast('info', `Draining ${title()} (job ${String(answer.job_id).slice(0, 8)}). Follow it in the Tasks tab.`);
  });
  const applyLock = (value) => run('lock', async () => {
    await nodesConnect.setEgressLock(props.id, value);
    setLock(value);
    setDialog(null);
    toast('ok', `Egress lock of ${title()} is now ${value === 'strict' ? 'strict' : 'exit locally'}.`);
    summary.refresh();
  });
  const chooseLock = (value) => {
    if (value === currentLock()) return;
    if (value === 'strict') { setLock(null); setDialog('lock'); } else applyLock(value);
  };
  const remove = () => run('delete', async () => {
    await nodesApi.removeJoined(props.id);
    setDialog(null);
    toast('ok', `Deleted node ${title()}.`);
    navigate(paths.home());
  });

  const actions = (
    <>
      <Show when={isOperator()}>
        <button type="button" class="primary" onClick={() => navigate(`${paths.menu('models')}?deploy=1&node=${encodeURIComponent(name())}`)}>Deploy model here</button>
        <button type="button" class="secondary" disabled={busy() === 'probe'} onClick={probe}>Probe</button>
        <button type="button" class="secondary" onClick={() => { setTarget(siblings()[0]?.name || ''); setDialog('drain'); }}>Drain…</button>
        <span class="inv-lock" title="How this node reaches the internet">
          <span class="muted small">Egress lock</span>
          <Segmented
            value={currentLock()}
            disabled={busy() === 'lock'}
            onChange={chooseLock}
            options={[{ value: 'strict', label: 'Strict' }, { value: 'exit_local', label: 'Exit locally' }]}
          />
        </span>
      </Show>
      <Show when={role() === 'admin'}>
        <button
          type="button"
          class="danger"
          disabled={!summary.data()?.empty}
          title={summary.data()?.empty ? '' : 'A node can be deleted only when nothing runs on it and no model is installed.'}
          onClick={() => setDialog('delete')}
        >
          Delete node {title()}
        </button>
      </Show>
    </>
  );

  return (
    <section class="page inv-page">
      <Breadcrumb items={crumbs()} />
      <PageHeader
        title={title()}
        status={<StatusBadge status={STATUS_WORD[status()] || 'unknown'} label={status()} />}
        subtitle={name() && name() !== title() ? name() : undefined}
        actions={actions}
      />
      <Show when={summary.error()}><p class="error">{summary.error()}</p></Show>
      <Tabs tabs={tabs()} active={state.tab()} onChange={state.select} />
      <Show when={state.tab() === 'summary'}>
        <Show when={summary.data()} fallback={<p class="muted">Loading…</p>}>
          <NodeSummaryTab summary={summary.data()} />
        </Show>
      </Show>
      <Show when={state.tab() === 'sandboxes'}>
        <ScopedSandboxesTab load={() => nodesApi.sandboxes(props.id)} emptyHint="Nothing runs on this node yet." />
      </Show>
      <Show when={state.tab() === 'models'}>
        <ScopedModelsTab nodeName={name()} onDeploy={() => navigate(`${paths.menu('models')}?deploy=1&node=${encodeURIComponent(name())}`)} />
      </Show>
      <Show when={state.tab() === 'images'}><NodeImagesTab nodeId={props.id} /></Show>
      <Show when={state.tab() === 'traffic'}><NodeTrafficTab nodeId={props.id} /></Show>
      <Show when={state.tab() === 'logs'}><NodeLogsTab nodeId={props.id} /></Show>

      <ConfirmDialog
        open={dialog() === 'drain'}
        title={`Drain ${title()}`}
        onClose={() => setDialog(null)}
        actions={[
          { label: 'Cancel', onClick: () => setDialog(null) },
          { label: 'Stop all sandboxes', tone: 'secondary', onClick: () => drain(true) },
          { label: target() ? `Move all to ${target()}` : 'Move all', tone: 'primary', onClick: () => target() && drain(false) },
        ]}
      >
        <p>Every sandbox on {title()} is moved to another node or stopped. Model installs stay where they are.</p>
        <label class="field">
          <span class="muted small">Target node</span>
          <select value={target()} onChange={(event) => setTarget(event.currentTarget.value)} aria-label="Target node">
            <For each={siblings()}>{(n) => <option value={n.name}>{n.name}</option>}</For>
          </select>
        </label>
        <Show when={siblings().length === 0}><p class="warn-note">No other online node is available; you can only stop the sandboxes.</p></Show>
      </ConfirmDialog>

      <ConfirmDialog
        open={dialog() === 'lock'}
        title={`Make egress of ${title()} strict`}
        onClose={() => setDialog(null)}
        actions={[
          { label: 'Cancel', onClick: () => setDialog(null) },
          { label: 'Set strict', tone: 'primary', onClick: () => applyLock('strict') },
        ]}
      >
        <p>With a strict lock all traffic of this node leaves through the gateway. Sandboxes lose any direct route they had.</p>
      </ConfirmDialog>

      <ConfirmDialog
        open={dialog() === 'delete'}
        title={`Delete node ${title()}`}
        onClose={() => setDialog(null)}
        actions={[
          { label: 'Cancel', onClick: () => setDialog(null) },
          { label: `Delete node ${title()}`, tone: 'danger', onClick: remove },
        ]}
      >
        <p>This removes <b class="mono">{name()}</b> from the data center. The machine itself is not touched.</p>
      </ConfirmDialog>
    </section>
  );
}
