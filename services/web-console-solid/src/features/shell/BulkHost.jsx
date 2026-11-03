// BulkHost: mounted once by the Shell. Binds `ui.selection` to `BulkBar`, asks for the input an
// action needs, runs the call and points to the dock's Tasks tab.
import { For, Show, createResource, createSignal } from 'solid-js';
import BulkBar from '../../components/BulkBar.jsx';
import ConfirmDialog from '../../components/ConfirmDialog.jsx';
import { actionsFor, confirmText, needsDialog, paramsFor, paramsValid, EGRESS_MODES } from '../../components/bulkActions.js';
import { modeLabel } from '../sandboxes/overview.js';
import { useUi } from '../../state/ui.jsx';
import { useSession } from '../../state/session.jsx';
import * as bulk from '../../api/bulk.js';
import * as models from '../../api/models.js';
import * as nodesApi from '../../api/nodes.js';
import * as policies from '../../api/policies.js';

export default function BulkHost() {
  const ui = useUi();
  const session = useSession();
  // The action waiting for input, or null.
  const [pending, setPending] = createSignal(null);
  const [params, setParams] = createSignal({});
  const [busy, setBusy] = createSignal(false);

  const kind = () => ui.selection.kind();
  const count = () => ui.selection.ids().size;
  const role = () => session.identity()?.role;
  const actions = () => actionsFor(kind(), role());

  // Choices for the dialog selects, loaded only while a dialog needs them.
  const [policyList] = createResource(() => pending()?.needs === 'policy_id' || null, async () => (await policies.list()).items || []);
  const [nodeList] = createResource(() => pending()?.needs === 'node' || null, async () => (await nodesApi.list()).items || []);
  const [modelList] = createResource(() => pending()?.needs === 'model_key' || null, async () => (await models.catalog()).items || []);

  function choose(id) {
    const action = actions().find((entry) => entry.id === id);
    if (!action) return;
    setParams({});
    if (needsDialog(action)) setPending(action);
    else run(action);
  }

  function openTasks() {
    ui.dock.setTab('tasks');
    ui.dock.setOpen(true);
  }

  async function run(action) {
    const ids = [...ui.selection.ids()];
    const values = params();
    if (!paramsValid(action.id, values)) return;
    setBusy(true);
    try {
      if (action.id === 'probe_all') {
        let failed = 0;
        for (const id of ids) {
          try { await nodesApi.probe(id); } catch { failed += 1; }
        }
        ui.toast.show({ tone: failed ? 'bad' : 'ok', text: `Probed ${ids.length - failed} of ${ids.length} nodes.` });
      } else if (action.id === 'deploy_model') {
        const answer = await models.installBulk(values.model_key.trim(), { nodes: ids });
        ui.toast.show({ tone: 'ok', text: `Started: job ${answer.job_id}. Open the Tasks tab.` });
        openTasks();
      } else {
        const answer = await bulk.sandboxes(ids, action.id, paramsFor(action.id, values));
        ui.toast.show({ tone: 'ok', text: `Started: job ${answer.job_id} (${answer.total} sandboxes). Open the Tasks tab.` });
        openTasks();
      }
      setPending(null);
      ui.selection.clear();
    } catch (failure) {
      ui.toast.show({ tone: 'bad', text: failure.message || 'The bulk action failed.' });
    } finally {
      setBusy(false);
    }
  }

  const set = (key, value) => setParams({ ...params(), [key]: value });

  return (
    <>
      <BulkBar count={count()} kind={kind()} actions={actions()} onAction={choose} onClear={() => ui.selection.clear()} />
      <ConfirmDialog
        open={!!pending()}
        title={pending() ? confirmText(pending().id, count(), kind()) : ''}
        onClose={() => setPending(null)}
        actions={[
          { label: 'Cancel', tone: 'secondary', onClick: () => setPending(null) },
          {
            label: busy() ? 'Working…' : pending()?.label.replace('…', '') || 'Run',
            tone: pending()?.danger ? 'danger' : 'primary',
            onClick: () => !busy() && pending() && paramsValid(pending().id, params()) && run(pending()),
          },
        ]}
      >
        <Show when={pending()?.needs === 'mode'}>
          <label class="bulk-field">Egress mode
            <select value={params().mode || ''} onChange={(event) => set('mode', event.currentTarget.value)}>
              <option value="">Choose…</option>
              <For each={EGRESS_MODES}>{(mode) => <option value={mode}>{modeLabel(mode)} ({mode})</option>}</For>
            </select>
          </label>
        </Show>
        <Show when={pending()?.needs === 'policy_id'}>
          <label class="bulk-field">Policy
            <select value={params().policy_id || ''} onChange={(event) => set('policy_id', event.currentTarget.value)}>
              <option value="">Choose…</option>
              <For each={policyList() || []}>{(policy) => <option value={policy.id}>{policy.name}</option>}</For>
            </select>
          </label>
        </Show>
        <Show when={pending()?.needs === 'node'}>
          <label class="bulk-field">Target node
            <select value={params().node || ''} onChange={(event) => set('node', event.currentTarget.value)}>
              <option value="">Choose…</option>
              <For each={nodeList() || []}>{(node) => <option value={node.name}>{node.name}</option>}</For>
            </select>
          </label>
        </Show>
        <Show when={pending()?.needs === 'model_key'}>
          <label class="bulk-field">Model
            <input list="bulk-models" value={params().model_key || ''} placeholder="model key"
                   onInput={(event) => set('model_key', event.currentTarget.value)} />
            <datalist id="bulk-models"><For each={modelList() || []}>{(model) => <option value={model.key} />}</For></datalist>
          </label>
        </Show>
        <Show when={pending()?.needs === 'confirm'}>
          <label class="bulk-field">Type <b class="mono">delete</b> to confirm. Containers and state are removed.
            <input value={params().confirm || ''} autocomplete="off" onInput={(event) => set('confirm', event.currentTarget.value)} />
          </label>
        </Show>
        <Show when={pending()?.id === 'allow_all'}>
          <p>Every connection these sandboxes ask for is allowed for 15 minutes, then the old mode returns.</p>
        </Show>
      </ConfirmDialog>
    </>
  );
}
