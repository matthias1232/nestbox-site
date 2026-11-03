// A site (the "cluster" of the tree): header, Summary, Nodes and Sandboxes tabs (contract C1, C3).
import { For, Show, createMemo, createSignal } from 'solid-js';
import Breadcrumb from '../shell/Breadcrumb.jsx';
import PageHeader from '../shell/PageHeader.jsx';
import { treePathFor } from '../shell/treeFilter.js';
import { useInventory } from '../shell/useInventory.js';
import Tabs from '../../components/Tabs.jsx';
import StatTile from '../../components/StatTile.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import SelectTable from '../../components/SelectTable.jsx';
import ConfirmDialog from '../../components/ConfirmDialog.jsx';
import { useSession } from '../../state/session.jsx';
import { useUi } from '../../state/ui.jsx';
import { navigate, paths } from '../../routing.js';
import * as sandboxesApi from '../../api/sandboxes.js';
import * as tenancy from '../../api/tenancy.js';
import ScopedSandboxesTab from './ScopedSandboxesTab.jsx';
import { useTabState } from './useNodeSummary.js';
import { countBelow, findNode, gib, nodesBelow } from './inventoryTabs.js';
import '../../styles/inventory.css';

const NODE_STATUS = { online: 'running', degraded: 'degraded', offline: 'offline', unknown: 'unknown' };

// A table of tree nodes (kind 'node'); a row opens the node page. Also used by the data center page.
export function NodesTable(props) {
  const columns = [
    { key: 'name', label: 'Node', sortable: true, render: (row) => <span class="mono">{row.name}</span> },
    { key: 'status', label: 'Status', sortable: true, render: (row) => <StatusBadge status={NODE_STATUS[row.status] || 'unknown'} label={row.status} /> },
    { key: 'tunnel_state', label: 'Tunnel', render: (row) => row.tunnel_state || '–' },
    { key: 'gpu', label: 'GPU', render: (row) => (row.gpu ? 'yes' : 'no') },
    { key: 'sandboxes', label: 'Sandboxes', align: 'right', render: (row) => `${row.counts?.running ?? 0} / ${row.counts?.sandboxes ?? 0}` },
    { key: 'ram', label: 'Free RAM', align: 'right', render: (row) => <span class="mono">{gib(row.budget?.ram_free_bytes)}</span> },
    { key: 'vram', label: 'Free VRAM', align: 'right', render: (row) => <span class="mono">{gib(row.budget?.vram_free_bytes)}</span> },
  ];
  return (
    <SelectTable
      columns={columns}
      rows={props.rows}
      rowKey="id"
      onRowClick={(row) => navigate(paths.node(row.id))}
      empty={{ title: 'No nodes here', hint: props.hint || 'Add a node to run sandboxes and models.' }}
    />
  );
}

// Props: `id` (site id), `tab` (optional).
export default function SitePage(props) {
  const session = useSession();
  const ui = useUi();
  const inventory = useInventory();
  const role = () => session.identity()?.role || 'member';
  const state = useTabState('site', role, () => props.tab, () => paths.site(props.id));
  const [confirm, setConfirm] = createSignal(false);

  const site = createMemo(() => findNode(inventory.root(), 'site', props.id));
  const nodes = createMemo(() => nodesBelow(site()));
  const counts = createMemo(() => countBelow(site()));
  const nodeNames = createMemo(() => new Set(nodes().map((n) => n.name)));
  const crumbs = createMemo(() => {
    const trail = treePathFor(inventory.root(), 'site', props.id);
    return trail.length ? trail : [{ label: 'Site' }];
  });
  const empty = () => nodes().length === 0;

  async function removeSite() {
    try {
      await tenancy.removeSite(props.id);
      setConfirm(false);
      ui?.toast.show({ tone: 'ok', text: `Removed site ${site()?.name}.`, undo: null, ms: 6000 });
      navigate(paths.home());
    } catch (failure) {
      ui?.toast.show({ tone: 'bad', text: failure.message || 'The site could not be removed.', undo: null, ms: 8000 });
    }
  }

  const actions = (
    <>
      <Show when={role() === 'admin' || role() === 'operator'}>
        <button type="button" class="primary" onClick={() => navigate(`${paths.menu('nodes')}?add=1&site=${props.id}`)}>Add node</button>
      </Show>
      <Show when={role() === 'admin'}>
        <button type="button" class="danger" disabled={!empty()} title={empty() ? '' : 'Remove or move every node first.'} onClick={() => setConfirm(true)}>
          Remove site {site()?.name || ''}
        </button>
      </Show>
    </>
  );

  return (
    <section class="page inv-page">
      <Breadcrumb items={crumbs()} />
      <PageHeader
        title={site()?.name || 'Site'}
        status={<Show when={site()}><StatusBadge status={site().status === 'ok' ? 'running' : site().status === 'warn' ? 'degraded' : 'offline'} label={site().status} /></Show>}
        subtitle={site()?.site_kind ? `${site().site_kind} site` : undefined}
        actions={actions}
      />
      <Show when={inventory.error()}><p class="error">{inventory.error()}</p></Show>
      <Tabs tabs={state.tabs()} active={state.tab()} onChange={state.select} />
      <Show when={!site() && !inventory.error()}><p class="muted">Loading…</p></Show>
      <Show when={site()}>
        <Show when={state.tab() === 'summary'}>
          <div class="inv-tab">
            <section class="inv-tiles">
              <StatTile label="Nodes online" value={counts().online} unit={`/ ${counts().nodes}`} tone={counts().online < counts().nodes ? 'warn' : undefined} />
              <StatTile label="Sandboxes running" value={counts().running} unit={`/ ${counts().sandboxes}`} />
              <StatTile label="Allow-all" value={site().counts?.allow_all ?? 0} tone={site().counts?.allow_all ? 'warn' : undefined} />
              <StatTile label="Alerts" value={site().counts?.alerts ?? 0} tone={site().counts?.alerts ? 'bad' : undefined} />
            </section>
            <h2 class="eyebrow">Nodes</h2>
            <Show when={nodes().length > 0} fallback={<p class="muted">This site has no node yet. Use “Add node”.</p>}>
              <div class="inv-cards">
                <For each={nodes()}>
                  {(n) => (
                    <button type="button" class="card inv-node-card" onClick={() => navigate(paths.node(n.id))} aria-label={`Open node ${n.name}`}>
                      <span class="mono">{n.name}</span>
                      <StatusBadge status={NODE_STATUS[n.status] || 'unknown'} label={n.status} />
                      <span class="muted small">{n.counts?.running ?? 0} / {n.counts?.sandboxes ?? 0} sandboxes</span>
                      <Show when={n.budget}>
                        <span class="small">Free RAM <b class="mono">{gib(n.budget.ram_free_bytes)}</b></span>
                        <Show when={n.gpu}><span class="small">Free VRAM <b class="mono">{gib(n.budget.vram_free_bytes)}</b></span></Show>
                      </Show>
                    </button>
                  )}
                </For>
              </div>
            </Show>
          </div>
        </Show>
        <Show when={state.tab() === 'nodes'}>
          <div class="inv-tab"><NodesTable rows={nodes()} /></div>
        </Show>
        <Show when={state.tab() === 'sandboxes'}>
          <ScopedSandboxesTab
            load={() => sandboxesApi.list({ limit: 200 })}
            filter={(row) => nodeNames().has(row.node_name)}
            emptyHint="No sandbox runs on a node of this site."
          />
        </Show>
      </Show>

      <ConfirmDialog
        open={confirm()}
        title={`Remove site ${site()?.name || ''}`}
        onClose={() => setConfirm(false)}
        actions={[
          { label: 'Cancel', onClick: () => setConfirm(false) },
          { label: `Remove site ${site()?.name || ''}`, tone: 'danger', onClick: removeSite },
        ]}
      >
        <p>The site is empty. Removing it deletes its record and join tokens.</p>
      </ConfirmDialog>
    </section>
  );
}
