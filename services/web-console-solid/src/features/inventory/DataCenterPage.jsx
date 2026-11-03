// A data center: header, Summary with a topology, Sites & nodes, Sandboxes, Models, Policies,
// Permissions and Usage (contract C1, C3).
import { For, Show, createEffect, createMemo } from 'solid-js';
import Breadcrumb from '../shell/Breadcrumb.jsx';
import PageHeader from '../shell/PageHeader.jsx';
import { treePathFor } from '../shell/treeFilter.js';
import { useInventory } from '../shell/useInventory.js';
import Tabs from '../../components/Tabs.jsx';
import StatTile from '../../components/StatTile.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import TopologyCanvas from '../../components/TopologyCanvas.jsx';
import { useSession } from '../../state/session.jsx';
import { navigate, paths } from '../../routing.js';
import * as sandboxesApi from '../../api/sandboxes.js';
import PoliciesPage from '../policies/PoliciesPage.jsx';
import PermissionsTab from '../users/PermissionsTab.jsx';
import UsageView from '../usage/UsageView.jsx';
import ScopedSandboxesTab from './ScopedSandboxesTab.jsx';
import ScopedModelsTab from './ScopedModelsTab.jsx';
import { NodesTable } from './SitePage.jsx';
import { useTabState } from './useNodeSummary.js';
import { countBelow, findNode, nodesBelow } from './inventoryTabs.js';
import '../../styles/inventory.css';

const LAYERS = ['site', 'node'];

// The topology of a data center: one column of sites, one of their nodes.
function drawing(dc) {
  const nodes = [];
  const edges = [];
  for (const site of dc.children || []) {
    if (site.kind !== 'site') continue;
    nodes.push({ id: site.id, kind: 'site', label: site.name, status: site.status, layer: 'site', meta: { kind: site.site_kind } });
    for (const n of nodesBelow(site)) {
      nodes.push({ id: n.id, kind: 'node', label: n.name, status: n.status, layer: 'node', meta: { sandboxes: n.counts?.sandboxes ?? 0 } });
      edges.push({ from: site.id, to: n.id });
    }
  }
  return { nodes, edges };
}

// Props: `id` (data center id), `tab` (optional).
export default function DataCenterPage(props) {
  const session = useSession();
  const inventory = useInventory();
  const role = () => session.identity()?.role || 'member';
  const state = useTabState('dc', role, () => props.tab, () => paths.dc(props.id));

  const dc = createMemo(() => findNode(inventory.root(), 'dc', props.id));
  const counts = createMemo(() => countBelow(dc()));
  const nodes = createMemo(() => nodesBelow(dc()));
  const sites = createMemo(() => (dc()?.children || []).filter((c) => c.kind === 'site'));
  const picture = createMemo(() => (dc() ? drawing(dc()) : { nodes: [], edges: [] }));
  const crumbs = createMemo(() => {
    const trail = treePathFor(inventory.root(), 'dc', props.id);
    return trail.length ? trail : [{ label: 'Data center' }];
  });
  const foreign = () => !!session.activeEnvironment() && session.activeEnvironment() !== props.id
    && session.environments().some((e) => e.id === props.id);
  // Sandboxes, models and policies are read through the active data center, so this page makes it active.
  createEffect(() => {
    if (foreign()) session.switchEnvironment(props.id);
  });
  const sick = createMemo(() => nodes().filter((n) => n.status !== 'online'));
  const isOperator = () => role() === 'operator' || role() === 'admin';

  const actions = (
    <Show when={isOperator()}>
      <button type="button" class="secondary" onClick={() => navigate(`${paths.menu('tenants')}?add=site&dc=${props.id}`)}>Add site</button>
      <button type="button" class="secondary" onClick={() => navigate(`${paths.menu('nodes')}?add=1`)}>Add node</button>
      <button type="button" class="primary" onClick={() => navigate(`${paths.menu('sandboxes')}?new=1`)}>New sandbox</button>
    </Show>
  );

  return (
    <section class="page inv-page">
      <Breadcrumb items={crumbs()} />
      <PageHeader
        title={dc()?.name || 'Data center'}
        status={<Show when={dc()}><StatusBadge status={dc().status === 'ok' ? 'running' : dc().status === 'warn' ? 'degraded' : 'offline'} label={dc().status} /></Show>}
        subtitle={crumbs().length > 1 ? `Organization ${crumbs()[0].label}` : undefined}
        actions={actions}
      />
      <Show when={foreign()}><p class="notice">Switching the active data center to this one so its sandboxes, models and policies load.</p></Show>
      <Show when={inventory.error()}><p class="error">{inventory.error()}</p></Show>
      <Tabs tabs={state.tabs()} active={state.tab()} onChange={state.select} />
      <Show when={!dc() && !inventory.error()}><p class="muted">Loading…</p></Show>
      <Show when={dc()}>
        <Show when={state.tab() === 'summary'}>
          <div class="inv-tab">
            <section class="inv-tiles">
              <StatTile label="Sites" value={counts().sites} />
              <StatTile label="Nodes online" value={counts().online} unit={`/ ${counts().nodes}`} tone={counts().online < counts().nodes ? 'warn' : undefined} />
              <StatTile label="Sandboxes running" value={counts().running} unit={`/ ${counts().sandboxes}`} />
              <StatTile label="Alerts" value={dc().counts?.alerts ?? 0} tone={dc().counts?.alerts ? 'bad' : undefined} />
            </section>
            <Show when={picture().nodes.length > 0}>
              <TopologyCanvas
                label="Data center topology"
                nodes={picture().nodes}
                edges={picture().edges}
                layers={LAYERS}
                selectedId={null}
                onSelect={() => {}}
                onAction={(id, action) => {
                  if (action !== 'open') return;
                  const hit = picture().nodes.find((n) => n.id === id);
                  navigate(hit?.kind === 'site' ? paths.site(id) : paths.node(id));
                }}
                height={360}
              />
            </Show>
            <h2 class="eyebrow">Health</h2>
            <Show when={sick().length > 0} fallback={<p class="muted">Every node is online.</p>}>
              <ul class="inv-health">
                <For each={sick()}>
                  {(n) => (
                    <li>
                      <StatusBadge status={n.status === 'degraded' ? 'degraded' : n.status === 'offline' ? 'offline' : 'unknown'} label={n.status} />
                      <button type="button" class="ghost" onClick={() => navigate(paths.node(n.id))}>{n.name}</button>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </div>
        </Show>
        <Show when={state.tab() === 'sites'}>
          <div class="inv-tab">
            <Show when={sites().length > 0} fallback={<p class="muted">No site yet. Use “Add site”.</p>}>
              <For each={sites()}>
                {(site) => (
                  <section class="inv-group">
                    <h2>
                      <button type="button" class="ghost" onClick={() => navigate(paths.site(site.id))}>{site.name}</button>
                      <Show when={site.site_kind}><span class="badge muted"><i />{site.site_kind}</span></Show>
                    </h2>
                    <NodesTable rows={nodesBelow(site)} hint="This site has no node yet." />
                  </section>
                )}
              </For>
            </Show>
          </div>
        </Show>
        <Show when={state.tab() === 'sandboxes'}>
          <ScopedSandboxesTab load={() => sandboxesApi.list({ limit: 200 })} emptyHint="Create the first sandbox with “New sandbox”." />
        </Show>
        <Show when={state.tab() === 'models'}>
          <ScopedModelsTab onDeploy={() => navigate(paths.menu('models'))} />
        </Show>
        <Show when={state.tab() === 'policies'}><PoliciesPage /></Show>
        <Show when={state.tab() === 'permissions'}><PermissionsTab scopeType="environment" scopeId={props.id} /></Show>
        <Show when={state.tab() === 'usage'}><UsageView defaultGroup="site" /></Show>
      </Show>
    </section>
  );
}
