// One model (`/models/<key>`): its installs, requirements, grants and clusters, with the actions on top.
import { For, Show, createMemo, createSignal, onMount } from 'solid-js';
import '../../styles/models.css';
import Breadcrumb from '../shell/Breadcrumb.jsx';
import PageHeader from '../shell/PageHeader.jsx';
import Tabs from '../../components/Tabs.jsx';
import Empty from '../../components/Empty.jsx';
import Meter from '../../components/Meter.jsx';
import { gib } from '../../components/ResourceBars.jsx';
import FitBadge from '../../components/FitBadge.jsx';
import * as models from '../../api/models.js';
import * as clusterApi from '../../api/clusters.js';
import * as modelGrants from '../../api/modelGrants.js';
import { navigate, paths } from '../../routing.js';
import ModelGrantsSection from './ModelGrantsSection.jsx';
import InstallsTable from './InstallsTable.jsx';
import BulkDeployWizard from './BulkDeployWizard.jsx';
import { useModels } from './useModels.js';

const TABS = [
  { id: 'installs', label: 'Installs' },
  { id: 'requirements', label: 'Requirements' },
  { id: 'grants', label: 'Grants' },
  { id: 'clusters', label: 'Clusters' },
];

// Props: id (the model key from the path).
export default function ModelPage(props) {
  const data = useModels(() => undefined);
  const [tab, setTab] = createSignal('installs');
  const [wizard, setWizard] = createSignal(false);
  const [clusterRows, setClusterRows] = createSignal([]);
  const [notice, setNotice] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [makeOpen, setMakeOpen] = createSignal(false);
  const [clusterName, setClusterName] = createSignal('');
  const [strategy, setStrategy] = createSignal('round_robin');

  const model = () => data.items().find((m) => m.key === props.id);
  const rows = () => data.installsByModel()[props.id] || [];

  async function loadClusters() {
    try {
      setClusterRows(((await clusterApi.list()).items || []).filter((c) => c.model_key === props.id));
    } catch (failure) {
      setError(failure.message);
    }
  }
  onMount(() => {
    data.refresh(false);
    loadClusters();
  });

  async function change(row, patch, label) {
    setError(null);
    try {
      const result = await models.updateInstall(row.model_key, { node: row.node_name, ...patch });
      setNotice(`${row.node_name}: ${label}. ${result.warning || ''}`.trim());
      await data.refresh(false);
    } catch (failure) {
      setError(failure.message);
    }
  }
  async function uninstall(row) {
    if (!window.confirm(`Uninstall ${row.model_name || row.model_key} from ${row.node_name}?`)) return;
    setError(null);
    try {
      await models.uninstall(row.model_key, row.node_name);
      setNotice(`${row.model_name || row.model_key} removed from ${row.node_name}.`);
      await data.refresh(false);
    } catch (failure) {
      setError(failure.message);
    }
  }
  // Create a cluster from every installed copy of this model (contract C7).
  async function createCluster() {
    setError(null);
    try {
      const made = await clusterApi.create({
        name: clusterName().trim(), model_key: props.id, strategy: strategy(),
        install_ids: installed().map((r) => r.id),
      });
      setMakeOpen(false);
      setNotice(`Cluster ${made.name || clusterName()} created.`);
      await loadClusters();
    } catch (failure) {
      setError(failure.message);
    }
  }
  async function warmAll() {
    setError(null);
    try {
      await modelGrants.warm(props.id);
      setNotice('Warm-up requested: the model is loaded and answers one small request.');
    } catch (failure) {
      setError(failure.message);
    }
  }

  const installed = createMemo(() => rows().filter((r) => r.state === 'installed'));
  const bars = () => model()?.fit?.bars || [];

  return (
    <section class="page model-page">
      <Breadcrumb items={[{ label: 'Models', path: paths.menu('models') }, { label: model()?.name || props.id }]} />
      <PageHeader
        title={model()?.name || props.id}
        subtitle={model() ? [model().family, model().parameters, model().context_length ? `${Math.round(model().context_length / 1024)}K context` : null, model().license, model().artifact_url ? 'artefact set' : 'no artefact'].filter(Boolean).join(' · ') : undefined}
        actions={
          <>
            <button type="button" class="primary" onClick={() => setWizard(true)}>Deploy on nodes…</button>
            <Show when={installed().length >= 2}>
              <button type="button" class="secondary" onClick={() => { setWizard(false); setTab('clusters'); setMakeOpen(true); }}>Make a cluster…</button>
            </Show>
            <button type="button" class="secondary" disabled={!installed().length} onClick={warmAll}>Warm all</button>
          </>
        }
      />
      <Show when={error() || data.error()}><p class="error" role="alert">{error() || data.error()}</p></Show>
      <Show when={notice()}><p class="notice">{notice()}</p></Show>
      <Show when={wizard()}>
        <BulkDeployWizard models={data.items()} modelKey={props.id} onClose={() => setWizard(false)} onDone={() => { data.refresh(false); loadClusters(); }} />
      </Show>
      <Show when={!model() && !data.busy()}>
        <Empty title="No such model" hint="The catalogue has no model with this key." action={<button type="button" class="secondary" onClick={() => navigate(paths.menu('models'))}>Back to models</button>} />
      </Show>
      <Tabs tabs={TABS} active={tab()} onChange={setTab} />
      <Show when={tab() === 'installs'}>
        <InstallsTable rows={rows()} model={model()} onChange={change} onUninstall={uninstall} onChanged={() => data.refresh(false)} />
      </Show>
      <Show when={tab() === 'requirements'}>
        <Show when={model()} fallback={<p class="muted">Loading…</p>}>
          <div class="requirements">
            <p class="muted small">The server's numbers for the node chosen on the Models page ("Fit against").</p>
            <FitBadge fit={model().fit || {}} />
            <Show when={bars().length} fallback={<p class="muted">No requirement bars for this model.</p>}>
              <For each={bars()}>
                {(bar) => (
                  <Meter
                    label={bar.resource}
                    value={bar.available || 0}
                    max={Math.max(bar.recommended || 0, bar.minimum || 0, bar.available || 0, 1)}
                    promised={bar.minimum}
                    text={`${gib(bar.available || 0)} available · min ${gib(bar.minimum || 0)} · rec ${gib(bar.recommended || bar.minimum || 0)}`}
                  />
                )}
              </For>
            </Show>
          </div>
        </Show>
      </Show>
      <Show when={tab() === 'grants'}><ModelGrantsSection /></Show>
      <Show when={tab() === 'clusters'}>
        <Show when={makeOpen()}>
          <form class="bulk-cluster" onSubmit={(event) => { event.preventDefault(); createCluster(); }}>
            <p class="small">Group the {installed().length} installed copies into one provider.</p>
            <label>Cluster name
              <input required value={clusterName()} onInput={(event) => setClusterName(event.currentTarget.value)} />
            </label>
            <label>Strategy
              <select value={strategy()} onChange={(event) => setStrategy(event.currentTarget.value)}>
                <option value="round_robin">Round robin</option>
                <option value="least_loaded">Least loaded</option>
                <option value="failover">Failover</option>
              </select>
            </label>
            <button type="submit" class="primary" disabled={!clusterName().trim() || installed().length < 2}>Create cluster</button>
            <button type="button" class="ghost" onClick={() => setMakeOpen(false)}>Cancel</button>
          </form>
        </Show>
        <Show when={clusterRows().length} fallback={<Empty title="No cluster for this model" hint="Deploy on two or more nodes and tick “Make these a cluster”." action={<button type="button" class="primary" onClick={() => setWizard(true)}>Deploy on nodes…</button>} />}>
          <div class="cluster-cards">
            <For each={clusterRows()}>
              {(c) => (
                <a class="cluster-card" href={paths.cluster(c.id)} onClick={(event) => { event.preventDefault(); navigate(paths.cluster(c.id)); }}>
                  <strong>{c.name}</strong>
                  <span class="muted small">{c.strategy} · {c.healthy_members ?? 0}/{(c.members || []).length} healthy</span>
                </a>
              )}
            </For>
          </div>
        </Show>
      </Show>
    </section>
  );
}
