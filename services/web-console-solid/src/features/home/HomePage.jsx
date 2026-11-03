// Home: the whole Nestbox on one page — numbers, the map, health, tasks and the latest alerts.
// Members get a smaller version: their sandboxes, running count and alerts.
import { For, Show, createMemo, createSignal } from 'solid-js';
import '../../styles/home.css';
import PageHeader from '../shell/PageHeader.jsx';
import Breadcrumb from '../shell/Breadcrumb.jsx';
import { useInventory } from '../shell/useInventory.js';
import { useAlerts } from '../dock/useAlerts.js';
import { useSession } from '../../state/session.jsx';
import { useUi } from '../../state/ui.jsx';
import { navigate, paths } from '../../routing.js';
import { usePolling } from '../../components/Poll.js';
import * as jobsApi from '../../api/jobs.js';
import * as systemApi from '../../api/system.js';
import StatTile from '../../components/StatTile.jsx';
import TopologyCanvas from '../../components/TopologyCanvas.jsx';
import AlertPopup from '../../components/AlertPopup.jsx';
import Meter from '../../components/Meter.jsx';
import Empty from '../../components/Empty.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import HealthList from './HealthList.jsx';
import QuickActions from './QuickActions.jsx';
import { LAYERS, graphFromTree, health, pathFor, totals } from './homeStats.js';

export default function HomePage() {
  const session = useSession();
  const ui = useUi();
  const inventory = useInventory();
  const alerts = useAlerts();
  const [jobs, setJobs] = createSignal([]);
  const [selectedId, setSelectedId] = createSignal(null);
  // The appliance's free disk, read once a minute. A failure keeps the last answer and
  // never surfaces as an error: a warning about the disk is not worth a broken page.
  const [ready, setReady] = createSignal(null);

  usePolling(() => jobsApi.list(20), {
    everyMs: 15000,
    onData: (result) => setJobs(result.items || []),
    onError: () => {},
  });

  usePolling(() => systemApi.ready(), {
    everyMs: 60000,
    onData: (result) => setReady(result),
    onError: () => {},
  });

  const member = () => session.identity()?.role === 'member';
  // The home page is about the data center the user is in, not the whole organisation: the
  // topbar picker scopes every other page, so the numbers, the map and the empty state follow
  // it too (U06-04).
  const root = () => inventory.center();
  const stats = createMemo(() => totals(root()));
  const graph = createMemo(() => graphFromTree(root()));
  const problems = createMemo(() => health(root(), ready()));
  const running = createMemo(() => jobs().filter((job) => job.status === 'running' || job.status === 'queued' || job.status === 'pending'));
  const latest = () => alerts.open().slice(0, 5);
  const myBoxes = createMemo(() => {
    const boxes = [];
    const visit = (node) => {
      if (node.kind === 'sandbox') boxes.push(node);
      (node.children || []).forEach(visit);
    };
    if (root()) visit(root());
    return boxes;
  });

  const open = (kind, id) => {
    const path = pathFor(kind, id);
    if (path) navigate(path);
  };
  const showAlerts = () => {
    ui.dock.setTab('alerts');
    ui.dock.setOpen(true);
  };

  const alertRows = () => (
    <Show when={latest().length > 0} fallback={<p class="muted small">No refused connections waiting for an answer.</p>}>
      <div class="select-table-wrap">
        <table class="select-table">
          <thead>
            <tr><th>Destination</th><th>Sandbox</th><th>Channel</th><th>Seen</th><th>Answer</th></tr>
          </thead>
          <tbody>
            <For each={latest()}>
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
                  onOpenSandbox={() => navigate(paths.sandbox(alert.sandbox_id))}
                />
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );

  return (
    <section class="page home-page">
      <Breadcrumb items={[{ label: 'Overview' }]} />
      <PageHeader
        title={member() ? 'My overview' : 'Overview'}
        subtitle={root()?.name}
        actions={<QuickActions member={member()} onNavigate={navigate} />}
      />
      <Show when={inventory.error() && !root()}>
        <div class="notice">{inventory.error()}</div>
      </Show>
      <Show when={root()} fallback={<p class="muted">Loading…</p>}>
        <Show
          when={member()}
          fallback={
            <Show
              when={stats().nodes > 0}
              fallback={
                <Empty
                  title="No nodes yet"
                  hint="Add a node so sandboxes have somewhere to run."
                  action={
                    <>
                      <img class="home-art" src="/illustrations/empty-nodes.webp" alt="" onError={(event) => { event.currentTarget.style.display = 'none'; }} />
                      <button type="button" class="primary" onClick={() => navigate('/nodes?add=1')}>Add your first node</button>
                    </>
                  }
                />
              }
            >
              <div class="home-tiles">
                <StatTile label="Data centers" value={stats().dataCenters} />
                <StatTile label="Nodes" value={`${stats().nodesOnline}/${stats().nodes}`} unit="online" tone={stats().nodesOnline < stats().nodes ? 'warn' : undefined} />
                <StatTile label="Sandboxes" value={`${stats().running}/${stats().sandboxes}`} unit="running" />
                <StatTile label="GPU nodes" value={stats().gpuNodes} />
                <StatTile
                  label="Allow-all"
                  value={stats().allowAll}
                  tone={stats().allowAll > 0 ? 'warn' : undefined}
                  onClick={() => navigate('/sandboxes?allow_all=1')}
                />
                <StatTile
                  label="Alerts"
                  value={alerts.open().length}
                  tone={alerts.open().length > 0 ? 'bad' : undefined}
                  onClick={showAlerts}
                />
              </div>
              <div class="home-columns">
                <div class="home-map">
                  <h2 class="eyebrow">Infrastructure map</h2>
                  <TopologyCanvas
                    nodes={graph().nodes}
                    edges={graph().edges}
                    layers={LAYERS}
                    height={420}
                    selectedId={selectedId()}
                    onSelect={(id) => {
                      setSelectedId(id);
                      const node = graph().nodes.find((item) => item.id === id);
                      if (node) open(node.kind, node.id);
                    }}
                    onAction={(id, action) => {
                      const node = graph().nodes.find((item) => item.id === id);
                      if (node && action === 'open') open(node.kind, node.id);
                    }}
                  />
                </div>
                <div class="home-side">
                  <h2 class="eyebrow">Health</h2>
                  <HealthList rows={problems()} onOpen={(row) => open(row.kind, row.id)} />
                  <h2 class="eyebrow home-gap">Running tasks</h2>
                  <Show when={running().length > 0} fallback={<p class="muted small">No tasks running.</p>}>
                    <For each={running()}>
                      {(job) => (
                        <Meter
                          label={String(job.kind || 'job').replace(/_/g, ' ')}
                          value={Number(job.completed) || 0}
                          max={Number(job.total) || 0}
                          text={`${job.completed || 0}/${job.total || 0}${job.failed ? ` · ${job.failed} failed` : ''}`}
                        />
                      )}
                    </For>
                  </Show>
                </div>
              </div>
              <h2 class="eyebrow home-gap">Latest alerts</h2>
              {alertRows()}
            </Show>
          }
        >
          <div class="home-tiles">
            <StatTile label="Sandboxes" value={stats().sandboxes} />
            <StatTile label="Running" value={stats().running} />
            <StatTile label="Alerts" value={alerts.open().length} tone={alerts.open().length > 0 ? 'bad' : undefined} onClick={showAlerts} />
          </div>
          <h2 class="eyebrow home-gap">My sandboxes</h2>
          <Show
            when={myBoxes().length > 0}
            fallback={<Empty title="No sandboxes yet" hint="Ask an operator to create one for you." />}
          >
            <div class="home-boxes">
              <For each={myBoxes()}>
                {(box) => (
                  <button type="button" class="home-box" onClick={() => navigate(paths.box(box.id))}>
                    <strong>{box.name}</strong>
                    <StatusBadge status={box.status} />
                    <span class="muted small">{box.pack_key || 'sandbox'}</span>
                  </button>
                )}
              </For>
            </div>
          </Show>
          <h2 class="eyebrow home-gap">Latest alerts</h2>
          {alertRows()}
        </Show>
      </Show>
    </section>
  );
}
