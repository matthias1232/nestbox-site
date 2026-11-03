// Live logs: up to four panes side by side, each with its own source, and a red lane of refused
// destinations above them. `LogsBoard` is shared with the pop-out window (LogsPopout.jsx).
//
// Pane sources are plain strings saved in `ui.logs.panes()` (see logLoaders.js). Nothing is stored
// by this screen; it polls the same endpoints the old single log did.

import { For, Index, Show, createMemo, createSignal, onMount } from 'solid-js';
import '../../styles/logs.css';
import * as sandboxApi from '../../api/sandboxes.js';
import * as systemApi from '../../api/system.js';
import * as nodeApi from '../../api/nodes.js';
import { useUi } from '../../state/ui.jsx';
import { paths } from '../../routing.js';
import PageHeader from '../shell/PageHeader.jsx';
import Breadcrumb from '../shell/Breadcrumb.jsx';
import LogPane from './LogPane.jsx';
import RedLane from './RedLane.jsx';
import { sandboxIdsOf } from './logLoaders.js';
import { environmentUrl } from '../../api/client.js';

const MAX_PANES = 4;

// The source of a pane that can open on its own in a pop-out (route: all or one sandbox id).
function popRouteId(source) {
  if (source === 'all') return 'all';
  return source.startsWith('sandbox:') ? source.slice(8) : null;
}

// Open a pop-out window for `id` ('all' or a sandbox id).
function openPopout(id) {
  window.open(environmentUrl(paths.popLogs(id)), `nestbox-logs-${id}`, 'popup,width=1100,height=760');
}

// The panes and the red lane. props: panes (getter of source strings), setPanes, role, runAlerts, inWindow.
export function LogsBoard(props) {
  const [sandboxes, setSandboxes] = createSignal([]);
  const [nodes, setNodes] = createSignal([]);
  const [components, setComponents] = createSignal([]);
  const [error, setError] = createSignal(null);

  async function loadSources() {
    try {
      const page = await sandboxApi.list({ limit: 100 });
      setSandboxes((page.items || []).filter((item) => item.status === 'running'));
    } catch (failure) {
      setError(failure.message);
    }
    try {
      const answer = await nodeApi.list();
      const rows = Array.isArray(answer) ? answer : answer.items || answer.nodes || [];
      setNodes(rows.map((item) => ({ id: item.id, name: item.name })));
    } catch { /* nodes are a convenience; a member has none */ }
    try {
      const answer = await systemApi.components();
      const rows = Array.isArray(answer) ? answer : answer.components || answer.items || [];
      setComponents(rows.map((item) => item.service));
    } catch { /* platform components are for admins only */ }
  }
  onMount(loadSources);

  const names = createMemo(() => ({
    sandbox: Object.fromEntries(sandboxes().map((item) => [item.id, item.name])),
    node: Object.fromEntries(nodes().map((item) => [item.id, item.name])),
  }));
  const runningIds = () => sandboxes().map((item) => item.id);
  // Sandboxes whose alerts the lane shows: null (all) when no pane is sandbox-based.
  const laneIds = () => {
    const ids = new Set();
    for (const source of props.panes()) (sandboxIdsOf(source, runningIds()) || []).forEach((id) => ids.add(id));
    return [...ids];
  };
  const setSource = (index, value) => props.setPanes(props.panes().map((s, i) => (i === index ? value : s)));
  const close = (index) => props.setPanes(props.panes().filter((_, i) => i !== index));

  return (
    <>
      <RedLane sandboxIds={laneIds()} run={props.runAlerts} inWindow={props.inWindow} />
      <Show when={error()}><p class="error">{error()}</p></Show>
      <div class="log-grid" data-count={props.panes().length}>
        <Index each={props.panes()}>
          {(source, index) => (
            <LogPane
              source={source()}
              names={names()}
              sandboxes={sandboxes()}
              nodes={nodes()}
              components={components()}
              canClose={props.panes().length > 1}
              canPopOut={popRouteId(source()) !== null}
              onSource={(value) => setSource(index, value)}
              onClose={() => close(index)}
              onPopOut={() => openPopout(popRouteId(source()))}
            />
          )}
        </Index>
      </div>
    </>
  );
}

// Add a pane showing everything, up to the maximum.
export function addPane(panes, setPanes) {
  if (panes().length < MAX_PANES) setPanes([...panes(), 'all']);
}

export default function LogsPage(props) {
  const ui = useUi();
  if (ui.logs.panes().length === 0) ui.logs.setPanes(['all']);
  const panes = () => (ui.logs.panes().length ? ui.logs.panes() : ['all']);

  return (
    <section class="page logs-page">
      <Breadcrumb items={[{ label: 'Live logs' }]} />
      <PageHeader
        title="Live logs"
        subtitle="What is happening right now. Lines are merged by time and say who wrote them. Red lines are refusals and errors."
        actions={
          <>
            <button class="secondary" disabled={panes().length >= MAX_PANES}
                    onClick={() => addPane(panes, ui.logs.setPanes)}>Add pane</button>
            <button class="secondary" onClick={() => openPopout('all')}>Pop out all</button>
          </>
        }
      />
      <LogsBoard panes={panes} setPanes={ui.logs.setPanes} role={props.role} />
    </section>
  );
}
