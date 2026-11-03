// Deploy a model on one or many nodes: Model → Nodes → Options → Review → live progress.
// Nothing is written before Finish; the Review step lists the exact call that will be made.
import { For, Show, createMemo, createSignal, onMount } from 'solid-js';
import Wizard from '../../components/Wizard.jsx';
import FitBadge from '../../components/FitBadge.jsx';
import * as nodeApi from '../../api/nodes.js';
import * as models from '../../api/models.js';
import * as clusters from '../../api/clusters.js';
import * as modelGrants from '../../api/modelGrants.js';
import { useInventory } from '../shell/useInventory.js';
import NodePicker from './NodePicker.jsx';
import DeployProgress from './DeployProgress.jsx';
import {
  candidateNodes, bodyFor, nodesProblem, optionsProblem, selectFitting, reviewLines,
} from './bulkDeploy.js';

const STEPS = [
  { id: 'model', label: 'Model' },
  { id: 'nodes', label: 'Nodes' },
  { id: 'options', label: 'Options' },
  { id: 'review', label: 'Review' },
];

// Props: models (catalogue rows), modelKey (preselected key or null), nodeName (preselected node name
// or null), onClose(), onDone() (called when the job is final, so the page can refresh).
export default function BulkDeployWizard(props) {
  const inventory = useInventory();
  const [step, setStep] = createSignal(props.modelKey ? 'nodes' : 'model');
  const [modelKey, setModelKey] = createSignal(props.modelKey || '');
  const [modelText, setModelText] = createSignal('');
  const [nodes, setNodes] = createSignal([]);
  const [selected, setSelected] = createSignal(new Set());
  const [nodeText, setNodeText] = createSignal('');
  const [force, setForce] = createSignal(false);
  const [makeCluster, setMakeCluster] = createSignal(false);
  const [clusterName, setClusterName] = createSignal('');
  const [strategy, setStrategy] = createSignal('round_robin');
  const [grant, setGrant] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal(null);
  const [started, setStarted] = createSignal(null); // { job_id, targets }
  const [grantNote, setGrantNote] = createSignal(null);

  const model = () => (props.models || []).find((m) => m.key === modelKey());
  const allCands = createMemo(() => candidateNodes(nodes(), inventory.root(), { model: model() }));
  const cands = createMemo(() => candidateNodes(nodes(), inventory.root(), { model: model(), text: nodeText() }));
  const state = () => ({
    modelKey: modelKey(), cands: allCands(), selected: selected(), force: force(),
    makeCluster: makeCluster(), clusterName: clusterName(), strategy: strategy(), grant: grant(),
  });

  onMount(async () => {
    try {
      setNodes((await nodeApi.list()).items || []);
    } catch (failure) {
      setError(failure.message);
    }
  });
  // Preselect the node named in the URL once the candidates exist (one pass, then the user decides).
  let preselected = false;
  createMemo(() => {
    if (preselected || !props.nodeName || !allCands().length) return;
    const hit = allCands().find((c) => c.name === props.nodeName);
    if (hit) setSelected(new Set([hit.id]));
    preselected = true;
  });

  const listed = createMemo(() => {
    const needle = modelText().trim().toLowerCase();
    return (props.models || [])
      .filter((m) => !m.is_remote && (!needle || `${m.name} ${m.key}`.toLowerCase().includes(needle)))
      .slice(0, 60);
  });

  const canNext = () => {
    if (step() === 'model') return !!model();
    if (step() === 'nodes') return nodesProblem(allCands(), selected()) === null;
    if (step() === 'options') return optionsProblem(state()) === null;
    return true;
  };

  async function finish() {
    setBusy(true);
    setError(null);
    try {
      setStarted(await models.installBulk(modelKey(), bodyFor(state())));
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  // When the job is final: grant the model to the data center if asked (one grant, on the cluster's
  // profile when a cluster exists, else on the first install's profile).
  async function done(job) {
    props.onDone?.(job);
    if (!grant() || job.status === 'failed') return;
    try {
      let profile = null;
      if (job.result?.cluster_id) profile = (await clusters.get(job.result.cluster_id)).provider_profile_id;
      if (!profile) {
        const rows = (await models.installs()).items || [];
        profile = rows.find((r) => r.model_key === modelKey() && r.provider_profile_id)?.provider_profile_id;
      }
      if (!profile) throw new Error('no provider profile exists yet');
      await modelGrants.create({
        scope: 'environment', model_name: modelKey(), provider_profile_id: profile, is_default: false, keep_warm: false,
      });
      setGrantNote('Granted to the whole data center.');
    } catch (failure) {
      setGrantNote(`The deploy finished, but the grant failed: ${failure.message}`);
    }
  }

  return (
    <section class="bulk-deploy" aria-label="Deploy a model on many nodes">
      <header class="bulk-deploy-head">
        <h2>Deploy {model()?.name || 'a model'}</h2>
        <button type="button" class="ghost" onClick={() => props.onClose()}>Close</button>
      </header>
      <Show when={error()}><p class="error" role="alert">{error()}</p></Show>
      <Show
        when={!started()}
        fallback={
          <>
            <DeployProgress jobId={started().job_id} modelKey={modelKey()} targets={started().targets} onDone={done} />
            <Show when={grantNote()}><p class="notice">{grantNote()}</p></Show>
          </>
        }
      >
        <Wizard
          steps={STEPS} step={step()} onStep={setStep} canNext={canNext} onFinish={finish} busy={busy()}
          finishLabel={`Deploy on ${selected().size} node${selected().size === 1 ? '' : 's'}`}
        >
          <Show when={step() === 'model'}>
            <input
              type="search" placeholder="Search models…" aria-label="Search models"
              value={modelText()} onInput={(event) => setModelText(event.currentTarget.value)}
            />
            <ul class="model-pick">
              <For each={listed()}>
                {(m) => (
                  <li>
                    <label>
                      <input type="radio" name="deploy-model" checked={modelKey() === m.key} onChange={() => setModelKey(m.key)} />
                      <strong>{m.name}</strong>
                      <span class="muted small mono">{m.key}</span>
                      <span class="muted small">{m.parameters}</span>
                    </label>
                    <FitBadge fit={m.fit || {}} />
                  </li>
                )}
              </For>
            </ul>
          </Show>
          <Show when={step() === 'nodes'}>
            <NodePicker
              cands={cands()} selected={selected()} onChange={setSelected} text={nodeText()} onText={setNodeText}
            />
            <Show when={nodesProblem(allCands(), selected())}>
              <p class="muted small">{nodesProblem(allCands(), selected())}</p>
            </Show>
            <Show when={!selected().size}>
              <button type="button" class="secondary tiny" onClick={() => setSelected(selectFitting(allCands()))}>Select all that fit</button>
            </Show>
          </Show>
          <Show when={step() === 'options'}>
            <div class="bulk-options">
              <label class="check-row">
                <input type="checkbox" checked={force()} onChange={(event) => setForce(event.currentTarget.checked)} />
                Install on nodes that do not fit (force)
              </label>
              <label class="check-row">
                <input type="checkbox" checked={makeCluster()} onChange={(event) => setMakeCluster(event.currentTarget.checked)} />
                Make these a cluster
              </label>
              <Show when={makeCluster()}>
                <div class="bulk-cluster">
                  <label>Cluster name
                    <input value={clusterName()} onInput={(event) => setClusterName(event.currentTarget.value)} placeholder={`${modelKey()}-x${selected().size}`} />
                  </label>
                  <label>Strategy
                    <select value={strategy()} onChange={(event) => setStrategy(event.currentTarget.value)}>
                      <option value="round_robin">Round robin</option>
                      <option value="least_loaded">Least loaded</option>
                      <option value="failover">Failover</option>
                    </select>
                  </label>
                  <Show when={optionsProblem(state())}><p class="muted small">{optionsProblem(state())}</p></Show>
                </div>
              </Show>
              <label class="check-row">
                <input type="checkbox" checked={grant()} onChange={(event) => setGrant(event.currentTarget.checked)} />
                Grant to this data center after success
              </label>
            </div>
          </Show>
          <Show when={step() === 'review'}>
            <p>This will make the following calls, in this order:</p>
            <ol class="review-lines">
              <For each={reviewLines(state(), model()?.name)}>{(line) => <li>{line}</li>}</For>
            </ol>
            <p class="muted small">Nodes the server finds unfit or already installed are skipped and listed on the next screen.</p>
          </Show>
        </Wizard>
      </Show>
    </section>
  );
}
