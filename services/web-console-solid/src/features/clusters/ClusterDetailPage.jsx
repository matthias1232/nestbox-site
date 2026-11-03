// /clusters/<id>: one cluster's members and health, strategy, provider profile and the buttons to change them.
import { For, Show, createMemo, createSignal, onMount } from 'solid-js';
import PageHeader from '../shell/PageHeader.jsx';
import Breadcrumb from '../shell/Breadcrumb.jsx';
import SelectTable from '../../components/SelectTable.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import ConfirmDialog from '../../components/ConfirmDialog.jsx';
import HelpTip from '../../components/HelpTip.jsx';
import Empty from '../../components/Empty.jsx';
import { navigate, paths } from '../../routing.js';
import { useUi } from '../../state/ui.jsx';
import * as clusters from '../../api/clusters.js';
import * as models from '../../api/models.js';
import { useCluster } from './useClusters.js';
import { STRATEGIES, health, strategyLabel, canRemove, pickableInstalls, usersFromError } from './clusterHealth.js';
import '../../styles/clusters.css';

export default function ClusterDetailPage(props) {
  const ui = useUi();
  const data = useCluster(() => props.id);
  const cluster = data.cluster;
  const [installs, setInstalls] = createSignal([]);
  const [editing, setEditing] = createSignal(false);
  const [name, setName] = createSignal('');
  const [strategy, setStrategy] = createSignal('round_robin');
  const [adding, setAdding] = createSignal(false);
  const [confirmDelete, setConfirmDelete] = createSignal(false);
  const [blockedBy, setBlockedBy] = createSignal([]);
  const [message, setMessage] = createSignal(null);
  const [busy, setBusy] = createSignal(false);

  onMount(async () => {
    try { setInstalls((await models.installs()).items || []); } catch { setInstalls([]); }
  });

  const status = createMemo(() => (cluster() ? health(cluster()) : null));
  const pickable = createMemo(() => {
    const c = cluster();
    if (!c) return [];
    return pickableInstalls(installs(), c.model_key, (c.members || []).map((m) => m.install_id));
  });
  const toast = (tone, text) => ui && ui.toast.show({ tone, text, undo: null, ms: 6000 });

  // Run one change, report its failure in the page, and redraw.
  async function run(action, okText) {
    setBusy(true);
    setMessage(null);
    try {
      await action();
      if (okText) toast('ok', okText);
      await data.refresh();
      return true;
    } catch (failure) {
      setMessage(failure.message);
      return false;
    } finally {
      setBusy(false);
    }
  }

  const startEdit = () => { setName(cluster().name); setStrategy(cluster().strategy); setEditing(true); };
  const save = async () => {
    if (await run(() => clusters.update(props.id, { name: name().trim(), strategy: strategy() }), 'Cluster updated.')) setEditing(false);
  };
  const add = async (installId) => {
    if (await run(() => clusters.addMember(props.id, installId), 'Member added.')) setAdding(false);
  };
  const removeMember = (installId) => run(() => clusters.removeMember(props.id, installId), 'Member removed.');

  async function destroy() {
    setBusy(true);
    setMessage(null);
    try {
      await clusters.remove(props.id);
      toast('ok', `Cluster ${cluster().name} deleted.`);
      navigate(paths.menu('clusters'));
    } catch (failure) {
      setBlockedBy(usersFromError(failure));
      setMessage(failure.message);
      setConfirmDelete(false);
    } finally {
      setBusy(false);
    }
  }

  const memberColumns = () => [
    { key: 'node', label: 'Node', sortable: true },
    { key: 'state', label: 'State', render: (m) => <StatusBadge status={m.state} /> },
    { key: 'provider_status', label: 'Provider', render: (m) => <StatusBadge status={m.provider_status || 'unknown'} /> },
    { key: 'weight', label: 'Weight', align: 'right', render: (m) => <span class="mono">{m.weight}</span> },
    { key: 'endpoint', label: 'Endpoint', render: (m) => <code class="mono small">{m.endpoint}</code> },
    {
      key: 'remove', label: '',
      render: (m) => (
        <button
          type="button" class="danger tiny" disabled={busy() || !canRemove(cluster(), m.install_id)}
          title={canRemove(cluster(), m.install_id) ? undefined : 'A cluster keeps at least one member.'}
          aria-label={`Remove ${m.node} from the cluster`}
          onClick={() => removeMember(m.install_id)}
        >Remove</button>
      ),
    },
  ];

  return (
    <section class="page cluster-detail">
      <Breadcrumb
        items={[
          { label: 'Models', path: paths.menu('models') },
          { label: cluster() ? cluster().model_key : 'Model', path: cluster() ? paths.model(cluster().model_key) : undefined },
          { label: cluster() ? cluster().name : 'Cluster' },
        ]}
      />
      <Show when={cluster()} fallback={<p class="muted">{data.error() || 'Loading…'}</p>}>
        <PageHeader
          title={cluster().name}
          status={<span class={`badge ${status().tone}`}><i />{status().text}</span>}
          subtitle={cluster().endpoint}
          actions={
            <>
              <HelpTip anchor="model-cluster-round-robin#use-it-in-a-sandbox" />
              <button type="button" class="secondary" disabled={busy()} onClick={() => setAdding(!adding())}>Add member…</button>
              <button type="button" class="secondary" disabled={busy()} onClick={startEdit}>Rename or change strategy</button>
              <button type="button" class="danger" disabled={busy()} onClick={() => setConfirmDelete(true)}>{`Delete cluster ${cluster().name}`}</button>
            </>
          }
        />
        <Show when={message()}>
          <div class="warn-note" role="alert">
            <p>{message()}</p>
            <Show when={blockedBy().length > 0}>
              <p>Used by: <For each={blockedBy()}>{(n, i) => <span class="mono">{i() > 0 ? ', ' : ''}{n}</span>}</For></p>
            </Show>
          </div>
        </Show>

        <Show when={editing()}>
          <form class="card cluster-edit" onSubmit={(e) => { e.preventDefault(); save(); }}>
            <label class="cluster-field">
              <span class="eyebrow">Name</span>
              <input type="text" value={name()} onInput={(e) => setName(e.currentTarget.value)} />
            </label>
            <fieldset class="cluster-field">
              <legend class="eyebrow">Strategy</legend>
              <For each={STRATEGIES}>
                {(s) => (
                  <label class="cluster-radio">
                    <input type="radio" name="edit-strategy" checked={strategy() === s.id} onChange={() => setStrategy(s.id)} />
                    <span><strong>{s.label}</strong> <span class="muted small">{s.hint}</span></span>
                  </label>
                )}
              </For>
            </fieldset>
            <div class="cluster-edit-actions">
              <button type="button" class="secondary" onClick={() => setEditing(false)}>Cancel</button>
              <button type="submit" class="primary" disabled={busy() || !name().trim()}>Save changes</button>
            </div>
          </form>
        </Show>

        <Show when={adding()}>
          <div class="card">
            <h2 class="eyebrow">Add a member</h2>
            <Show when={pickable().length > 0} fallback={<Empty title="No other installs of this model" hint="Install the model on another node, then add it here." />}>
              <ul class="cluster-picklist">
                <For each={pickable()}>
                  {(row) => (
                    <li>
                      <span>{row.node_name || row.node_id}</span>
                      <button type="button" class="secondary tiny" disabled={busy()} onClick={() => add(row.id)}>{`Add ${row.node_name || 'node'}`}</button>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </div>
        </Show>

        <div class="card">
          <h2 class="eyebrow">Members · {strategyLabel(cluster().strategy)}</h2>
          <SelectTable
            columns={memberColumns()}
            rows={cluster().members || []}
            rowKey="install_id"
            empty={{ title: 'No members', hint: 'Add an install of this model to serve requests.' }}
          />
        </div>

        <div class="card">
          <h2 class="eyebrow">How a sandbox uses it</h2>
          <p>
            Pick the provider profile <strong class="mono">{cluster().name}</strong> for a sandbox and ask for the model
            {' '}<strong class="mono">{cluster().model_key}</strong>. Requests go to <code class="mono">{cluster().endpoint}</code> and the platform picks the member.
          </p>
          <HelpTip anchor="model-cluster-round-robin#use-it-in-a-sandbox" />
        </div>

        <div class="card">
          <h2 class="eyebrow">Usage</h2>
          <p class="muted">
            Not shown yet: the usage report cannot be filtered by provider, so a cluster's own figures are not available.
            The Usage page lists it under the provider named {cluster().name}.
          </p>
        </div>

        <ConfirmDialog
          open={confirmDelete()}
          title={`Delete cluster ${cluster().name}?`}
          onClose={() => setConfirmDelete(false)}
          actions={[
            { label: 'Keep cluster', tone: 'secondary', onClick: () => setConfirmDelete(false) },
            { label: `Delete cluster ${cluster().name}`, tone: 'danger', onClick: destroy },
          ]}
        >
          The cluster and its provider profile are removed. The installs on the nodes stay. If a sandbox still uses it, the delete is refused and the sandboxes are listed.
        </ConfirmDialog>
      </Show>
    </section>
  );
}
