// /clusters: every model cluster, with health, and the dialog that creates one.
import { Show, createSignal, onMount } from 'solid-js';
import PageHeader from '../shell/PageHeader.jsx';
import Breadcrumb from '../shell/Breadcrumb.jsx';
import SelectTable from '../../components/SelectTable.jsx';
import HelpTip from '../../components/HelpTip.jsx';
import { navigate, paths } from '../../routing.js';
import { useUi } from '../../state/ui.jsx';
import * as models from '../../api/models.js';
import NewClusterDialog from './NewClusterDialog.jsx';
import { useClusterList } from './useClusters.js';
import { health, strategyLabel } from './clusterHealth.js';
import '../../styles/clusters.css';

// Copy text to the clipboard; a refusal (no permission) is ignored, the text stays selectable.
async function copyText(text) {
  try { await navigator.clipboard.writeText(text); } catch { /* the endpoint stays visible to select */ }
}

export default function ClustersPage() {
  const ui = useUi();
  const list = useClusterList();
  const [installs, setInstalls] = createSignal([]);
  const [creating, setCreating] = createSignal(false);

  onMount(async () => {
    try { setInstalls((await models.installs()).items || []); } catch { setInstalls([]); }
  });

  const columns = [
    { key: 'name', label: 'Name', sortable: true },
    { key: 'model_key', label: 'Model', sortable: true, render: (c) => <span class="mono">{c.model_key}</span> },
    { key: 'strategy', label: 'Strategy', sortable: true, render: (c) => strategyLabel(c.strategy) },
    { key: 'health', label: 'Health', render: (c) => { const h = health(c); return <span class={`badge ${h.tone}`}><i />{h.text}</span>; } },
    { key: 'members', label: 'Members', align: 'right', render: (c) => (c.members || []).length },
    {
      key: 'endpoint', label: 'Endpoint',
      render: (c) => (
        <span class="cluster-endpoint">
          <code class="mono small">{c.endpoint}</code>
          <button type="button" class="ghost tiny" aria-label={`Copy endpoint of ${c.name}`} onClick={(e) => { e.stopPropagation(); copyText(c.endpoint); }}>Copy</button>
        </span>
      ),
    },
    { key: 'open', label: '', render: (c) => <button type="button" class="secondary tiny" onClick={() => navigate(paths.cluster(c.id))}>Open</button> },
  ];

  const created = (made) => {
    setCreating(false);
    if (ui) ui.toast.show({ tone: 'ok', text: `Cluster ${made.name} created.`, undo: null, ms: 6000 });
    list.refresh();
    if (made.id) navigate(paths.cluster(made.id));
  };

  return (
    <section class="page clusters-page">
      <Breadcrumb items={[{ label: 'Models', path: paths.menu('models') }, { label: 'Clusters' }]} />
      <PageHeader
        title="Model clusters"
        subtitle="Several installs of one model behind one endpoint."
        actions={
          <>
            <HelpTip anchor="model-cluster-round-robin#use-it-in-a-sandbox" />
            <button type="button" class="primary" onClick={() => setCreating(true)}>New cluster…</button>
          </>
        }
      />
      <Show when={list.error()}><p class="warn-note" role="alert">{list.error()}</p></Show>
      <Show when={list.loaded()} fallback={<p class="muted">Loading…</p>}>
        <SelectTable
          columns={columns}
          rows={list.items()}
          rowKey="id"
          onRowClick={(c) => navigate(paths.cluster(c.id))}
          empty={{
            title: 'No clusters yet',
            hint: 'Group installs of the same model so one endpoint spreads the requests.',
            action: <button type="button" class="primary" onClick={() => setCreating(true)}>New cluster…</button>,
          }}
        />
      </Show>
      <Show when={creating()}>
        <NewClusterDialog installs={installs()} onClose={() => setCreating(false)} onCreated={created} />
      </Show>
    </section>
  );
}
