// New sandbox as a four-step wizard: What, Where, Access, Review (package F24).
//
// Nothing is written before Finish. The Review step lists every call that will be made, in order:
// the create first, then the follow-ups (model grants, policy, mounts). The rules live in
// `createWizard.js`; the older `CreateSandboxForm.jsx` and `useCreateSandbox.js` stay as they are.

import { For, Show, createMemo, createSignal, onMount } from 'solid-js';
import Wizard from '../../components/Wizard.jsx';
import SelectTable from '../../components/SelectTable.jsx';
import Empty from '../../components/Empty.jsx';
import { gib } from '../../components/ResourceBars.jsx';
import { navigate, paths } from '../../routing.js';
import { useUi } from '../../state/ui.jsx';
import * as nodesApi from '../../api/nodes.js';
import * as packsApi from '../../api/packs.js';
import * as recipesApi from '../../api/recipes.js';
import * as sandboxesApi from '../../api/sandboxes.js';
import * as modelGrantsApi from '../../api/modelGrants.js';
import * as policiesApi from '../../api/policies.js';
import * as persistentApi from '../../api/persistent.js';
import * as mountsApi from '../../api/mounts.js';
import {
  EGRESS_MODES, blankState, bodyFor, createCall, followUps, hasOwnLogin, leastLoaded, nodeHint, stepsFor, validate,
} from './createWizard.js';
import '../../styles/create-wizard.css';

const API = {
  'sandboxes.create': (body) => sandboxesApi.create(body),
  'recipes.instantiate': (id, name) => recipesApi.instantiate(id, name),
  'modelGrants.create': (body) => modelGrantsApi.create(body),
  'policies.assignPolicy': (id, policyId) => policiesApi.assignPolicy(id, policyId),
  'mounts.put': (id, body) => mountsApi.put(id, body),
};

// A list answer is either an array or `{ items }`.
const itemsOf = (answer) => (Array.isArray(answer) ? answer : (answer && answer.items) || []);

// Props: onCancel(), onCreated(sandbox) (both optional; the list page closes its drawer from them).
export default function CreateSandboxWizard(props) {
  const ui = useUi();
  const [state, setState] = createSignal(blankState());
  const [step, setStep] = createSignal('what');
  const [packs, setPacks] = createSignal([]);
  const [recipes, setRecipes] = createSignal([]);
  const [nodes, setNodes] = createSignal([]);
  const [grants, setGrants] = createSignal([]);
  const [policies, setPolicies] = createSignal([]);
  const [roots, setRoots] = createSignal([]);
  const [loadNotes, setLoadNotes] = createSignal([]);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal(null);
  const [done, setDone] = createSignal(null);

  const patch = (changes) => setState({ ...state(), ...changes });
  const steps = createMemo(() => stepsFor(state().source));
  const problems = createMemo(() => validate(step(), state()));
  const canNext = () => step() === 'review' || problems().length === 0;

  onMount(async () => {
    const asks = [
      ['agent packs', packsApi.agentPacks()],
      ['recipes', recipesApi.list()],
      ['nodes', nodesApi.list()],
      ['models', modelGrantsApi.list()],
      ['policies', policiesApi.list()],
      ['persistent roots', persistentApi.list()],
    ];
    const answers = await Promise.allSettled(asks.map((a) => a[1]));
    const notes = [];
    const take = (index) => {
      const answer = answers[index];
      if (answer.status === 'fulfilled') return itemsOf(answer.value);
      notes.push(`The ${asks[index][0]} could not be read (${answer.reason?.message || 'error'}).`);
      return [];
    };
    const packList = take(0);
    setPacks(packList);
    setRecipes(take(1));
    setNodes(take(2));
    setGrants(take(3));
    setPolicies(take(4));
    setRoots(take(5));
    setLoadNotes(notes);
    if (packList.length > 0 && !state().pack_key) patch({ pack_key: packList[0].key });
  });

  // The models an operator may hand out: the environment's own grants, one entry per name.
  const grantByName = createMemo(() => {
    const map = {};
    for (const grant of grants()) {
      if (grant.scope === 'environment' && !map[grant.model_name]) map[grant.model_name] = grant;
    }
    return map;
  });
  const modelNames = createMemo(() => Object.keys(grantByName()).sort());

  const chosenNode = createMemo(() => {
    if (state().node) return nodes().find((n) => n.name === state().node) || null;
    return leastLoaded(nodes(), state());
  });
  const rootsHere = createMemo(() => {
    const node = chosenNode();
    return roots().filter((root) => !root.archived_at && (!node || root.node_id === node.id));
  });

  const toggleModel = (name) => {
    const current = state().models;
    patch({ models: current.includes(name) ? current.filter((n) => n !== name) : [...current, name] });
  };
  const mountFor = (rootId) => state().mounts.find((m) => m.root_id === rootId);
  const toggleRoot = (root) => {
    if (mountFor(root.id)) patch({ mounts: state().mounts.filter((m) => m.root_id !== root.id) });
    else patch({ mounts: [...state().mounts, { root_id: root.id, mount_path: `/mnt/${root.slug}`, read_only: false }] });
  };
  const editMount = (rootId, changes) => patch({
    mounts: state().mounts.map((m) => (m.root_id === rootId ? { ...m, ...changes } : m)),
  });

  const nodeColumns = [
    { key: 'name', label: 'Node', sortable: true, render: (n) => n.display_name || n.name },
    { key: 'site', label: 'Site', render: (n) => n.site_name || '—' },
    { key: 'free', label: 'Free RAM', align: 'right', render: (n) => (n.hardware_known ? gib(n.budget?.ram_free_bytes) : '—') },
    {
      key: 'bar', label: 'Used',
      render: (n) => {
        const total = n.budget?.ram_total_bytes || 0;
        const used = total > 0 ? Math.round(100 * (1 - (n.budget.ram_free_bytes || 0) / total)) : 0;
        return (
          <span class="cw-bar" role="img" aria-label={`${used} percent of memory in use`}>
            <i style={{ width: `${used}%` }} />
          </span>
        );
      },
    },
    {
      key: 'hint', label: 'For this sandbox',
      render: (n) => {
        const hint = nodeHint(n, state());
        return <span class={`badge ${hint.tone}`}><i />{hint.text || '—'}</span>;
      },
    },
  ];

  // The create call, then each follow-up in order. A failed follow-up stops the list and says which
  // calls are still open, because the sandbox exists by then.
  async function finish() {
    if (busy()) return;
    setBusy(true);
    setError(null);
    let sandbox = null;
    try {
      const first = createCall(state());
      sandbox = await API[first.call.api](...first.call.args);
    } catch (failure) {
      setError(failure.message);
      setBusy(false);
      return;
    }
    const failed = [];
    const calls = followUps(state(), sandbox.id, grantByName());
    for (let i = 0; i < calls.length; i += 1) {
      try {
        await API[calls[i].call.api](...calls[i].call.args);
      } catch (failure) {
        failed.push(`${calls[i].label}: ${failure.message}`);
        // Later calls do not depend on this one except mounts after policy; keep going and report all.
      }
    }
    setBusy(false);
    setDone({ sandbox, failed });
    ui.toast.show({
      tone: failed.length > 0 ? 'bad' : 'ok',
      text: failed.length > 0
        ? `Sandbox ${sandbox.name} created, ${failed.length} follow-up step(s) failed.`
        : `Sandbox ${sandbox.name} created.`,
      undo: null,
      ms: 6000,
    });
    if (props.onCreated) props.onCreated(sandbox);
  }

  const open = (path) => navigate(path);

  return (
    <div class="create-wizard">
      <Show when={loadNotes().length > 0}>
        <p class="warn-note">{loadNotes().join(' ')}</p>
      </Show>

      <Show
        when={!done()}
        fallback={
          <div class="cw-done">
            <h2>Sandbox “{done().sandbox.name}” created</h2>
            <Show when={done().failed.length > 0}>
              <div class="warn-note" role="alert">
                <b>These steps did not finish; apply them from the Admin Center:</b>
                <ul><For each={done().failed}>{(line) => <li>{line}</li>}</For></ul>
              </div>
            </Show>
            <div class="cw-actions">
              <button type="button" class="primary" onClick={() => open(paths.box(done().sandbox.id))}>Open workbox</button>
              <button type="button" class="secondary" onClick={() => open(paths.sandbox(done().sandbox.id))}>Open admin view</button>
              <Show when={props.onCancel}>
                <button type="button" class="ghost" onClick={() => props.onCancel()}>Close</button>
              </Show>
            </div>
          </div>
        }
      >
        <Wizard
          steps={steps()}
          step={steps().some((s) => s.id === step()) ? step() : 'what'}
          onStep={setStep}
          canNext={canNext}
          onFinish={finish}
          finishLabel="Create sandbox"
          busy={busy()}
        >
          <Show when={problems().length > 0 && step() !== 'review'}>
            <ul class="cw-problems" aria-live="polite">
              <For each={problems()}>{(line) => <li>{line}</li>}</For>
            </ul>
          </Show>

          {/* 1. What */}
          <Show when={step() === 'what'}>
            <h2>What should run?</h2>
            <label class="cw-field">
              <span>Name</span>
              <input type="text" value={state().name} placeholder="research-agent" onInput={(e) => patch({ name: e.currentTarget.value })} />
            </label>
            <div class="cw-cards" role="radiogroup" aria-label="Source">
              <button type="button" role="radio" aria-checked={state().source === 'pack'} class="cw-card" classList={{ on: state().source === 'pack' }} onClick={() => patch({ source: 'pack' })}>
                <b>Agent pack</b><span class="muted small">A ready-made agent on the pack's default image.</span>
              </button>
              <button type="button" role="radio" aria-checked={state().source === 'recipe'} class="cw-card" classList={{ on: state().source === 'recipe' }} onClick={() => patch({ source: 'recipe' })}>
                <b>Recipe</b><span class="muted small">An image you built; it keeps its own node and limits.</span>
              </button>
              <button type="button" role="radio" aria-checked={state().source === 'image'} class="cw-card" classList={{ on: state().source === 'image' }} onClick={() => patch({ source: 'image' })}>
                <b>OCI image</b><span class="muted small">Any image, pinned by its sha256 digest.</span>
              </button>
            </div>

            <Show when={state().source !== 'recipe'}>
              <h3 class="eyebrow">{state().source === 'image' ? 'Agent pack on the image' : 'Agent pack'}</h3>
              <Show when={packs().length > 0} fallback={<Empty title="No agent packs" hint="The server returned none." />}>
                <div class="cw-cards" role="radiogroup" aria-label="Agent pack">
                  <For each={packs()}>
                    {(pack) => (
                      <button type="button" role="radio" aria-checked={state().pack_key === pack.key} class="cw-card" classList={{ on: state().pack_key === pack.key }} onClick={() => patch({ pack_key: pack.key })}>
                        <b>{pack.name || pack.label || pack.key}</b>
                        <span class="muted small">{pack.description || pack.key}</span>
                        <Show when={hasOwnLogin(pack.key)}><span class="badge info"><i />own login possible</span></Show>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
              <Show when={hasOwnLogin(state().pack_key)}>
                <h3 class="eyebrow">Login</h3>
                <div class="cw-cards" role="radiogroup" aria-label="Login mode">
                  <button type="button" role="radio" aria-checked={state().login === 'gateway'} class="cw-card" classList={{ on: state().login === 'gateway' }} onClick={() => patch({ login: 'gateway' })}>
                    <b>Through the gateway</b><span class="muted small">Uses the appliance's model key. Nothing to sign in to.</span>
                  </button>
                  <button type="button" role="radio" aria-checked={state().login === 'own_account'} class="cw-card" classList={{ on: state().login === 'own_account' }} onClick={() => patch({ login: 'own_account' })}>
                    <b>Own account</b><span class="muted small">You sign in yourself inside the sandbox.</span>
                  </button>
                </div>
              </Show>
            </Show>

            <Show when={state().source === 'image'}>
              <label class="cw-field">
                <span>Image reference</span>
                <input type="text" class="mono" value={state().image} placeholder="registry.example/team/tool" onInput={(e) => patch({ image: e.currentTarget.value })} />
              </label>
              <label class="cw-field">
                <span>Digest</span>
                <input type="text" class="mono" value={state().image_digest} placeholder="sha256:…" onInput={(e) => patch({ image_digest: e.currentTarget.value })} />
              </label>
            </Show>

            <Show when={state().source === 'recipe'}>
              <h3 class="eyebrow">Recipe</h3>
              <SelectTable
                columns={[
                  { key: 'name', label: 'Recipe', sortable: true },
                  { key: 'status', label: 'State', render: (r) => r.status || r.build_status || '—' },
                  { key: 'pick', label: '', render: (r) => <input type="radio" name="cw-recipe" aria-label={`Use recipe ${r.name}`} checked={state().recipe_id === r.id} onChange={() => patch({ recipe_id: r.id })} /> },
                ]}
                rows={recipes()}
                rowKey="id"
                onRowClick={(r) => patch({ recipe_id: r.id })}
                empty={{ title: 'No recipes yet', hint: 'Build one under Recipes, or pick an agent pack.' }}
              />
            </Show>
          </Show>

          {/* 2. Where */}
          <Show when={step() === 'where'}>
            <h2>Where should it run?</h2>
            <p class="muted">The active data center is used. Pick a node, or let Nestbox choose the one with the most free memory that fits.</p>
            <button type="button" class="cw-card" classList={{ on: !state().node }} aria-pressed={!state().node} onClick={() => patch({ node: '' })}>
              <b>Let Nestbox pick (least loaded)</b>
              <span class="muted small">
                {leastLoaded(nodes(), state()) ? `Right now: ${leastLoaded(nodes(), state()).name}` : 'No node fits the numbers below yet; the server will say so.'}
              </span>
            </button>
            <SelectTable
              columns={nodeColumns}
              rows={nodes()}
              rowKey="name"
              selectable={false}
              onRowClick={(n) => patch({ node: n.name })}
              empty={{ title: 'No nodes', hint: 'The node list is empty or could not be read; the local node is used.' }}
            />
            <Show when={state().node}>
              <p class="small">Chosen node: <b class="mono">{state().node}</b></p>
            </Show>
            <h3 class="eyebrow">Limits</h3>
            <div class="cw-grid">
              <label class="cw-field"><span>Memory (GiB)</span><input type="number" min="0.25" step="0.25" value={state().memory_gib} onInput={(e) => patch({ memory_gib: e.currentTarget.value })} /></label>
              <label class="cw-field"><span>CPU cores</span><input type="number" min="0.25" step="0.25" value={state().cpu} onInput={(e) => patch({ cpu: e.currentTarget.value })} /></label>
              <label class="cw-field"><span>PIDs</span><input type="number" min="1" step="1" value={state().pids} onInput={(e) => patch({ pids: e.currentTarget.value })} /></label>
              <label class="cw-field"><span>GPUs</span><input type="number" min="0" step="1" value={state().gpu_count} onInput={(e) => patch({ gpu_count: e.currentTarget.value })} /></label>
            </div>
          </Show>

          {/* 3. Access */}
          <Show when={step() === 'access'}>
            <h2>What may it reach?</h2>
            <h3 class="eyebrow">Models</h3>
            <Show when={modelNames().length > 0} fallback={<p class="muted">No environment-wide model grants exist, so there is nothing to copy to this sandbox.</p>}>
              <div class="cw-checks">
                <For each={modelNames()}>
                  {(name) => (
                    <label class="cw-check">
                      <input type="checkbox" checked={state().models.includes(name)} onChange={() => toggleModel(name)} />
                      <span class="mono">{name}</span>
                    </label>
                  )}
                </For>
              </div>
            </Show>

            <h3 class="eyebrow">Egress mode</h3>
            <div class="cw-cards" role="radiogroup" aria-label="Egress mode">
              <For each={EGRESS_MODES}>
                {(mode) => (
                  <button type="button" role="radio" aria-checked={state().egress_mode === mode.value} class="cw-card" classList={{ on: state().egress_mode === mode.value }} onClick={() => patch({ egress_mode: mode.value })}>
                    <b>{mode.label}</b><span class="muted small">{mode.help}</span>
                  </button>
                )}
              </For>
            </div>

            <h3 class="eyebrow">Policy</h3>
            <label class="cw-field">
              <span>Network policy</span>
              <select value={state().policy_id} onChange={(e) => patch({ policy_id: e.currentTarget.value })}>
                <option value="">Default (none assigned)</option>
                <For each={policies()}>{(policy) => <option value={policy.id}>{policy.name}</option>}</For>
              </select>
            </label>

            <h3 class="eyebrow">Persistent folders</h3>
            <Show when={rootsHere().length > 0} fallback={<p class="muted">No persistent roots on {chosenNode() ? chosenNode().name : 'any node'}.</p>}>
              <ul class="cw-roots">
                <For each={rootsHere()}>
                  {(root) => (
                    <li>
                      <label class="cw-check">
                        <input type="checkbox" checked={!!mountFor(root.id)} onChange={() => toggleRoot(root)} />
                        <span class="mono">{root.slug}</span>
                        <span class="muted small">{root.kind}</span>
                      </label>
                      <Show when={mountFor(root.id)}>
                        <input type="text" class="mono" aria-label={`Mount path for ${root.slug}`} value={mountFor(root.id).mount_path} onInput={(e) => editMount(root.id, { mount_path: e.currentTarget.value })} />
                        <label class="cw-check"><input type="checkbox" checked={mountFor(root.id).read_only} onChange={(e) => editMount(root.id, { read_only: e.currentTarget.checked })} /><span>read-only</span></label>
                      </Show>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </Show>

          {/* 4. Review */}
          <Show when={step() === 'review'}>
            <h2>Review</h2>
            <dl class="facts">
              <dt>Name</dt><dd>{state().name}</dd>
              <dt>Source</dt>
              <dd>
                {state().source === 'recipe'
                  ? `Recipe ${recipes().find((r) => r.id === state().recipe_id)?.name || state().recipe_id}`
                  : state().source === 'image'
                    ? `Image ${state().image}`
                    : `Pack ${state().pack_key}`}
              </dd>
              <Show when={state().source === 'image'}><dt>Digest</dt><dd class="mono">{state().image_digest}</dd></Show>
              <Show when={state().source !== 'recipe'}>
                <dt>Login</dt><dd>{hasOwnLogin(state().pack_key) && state().login === 'own_account' ? 'Own account' : 'Through the gateway'}</dd>
                <dt>Node</dt><dd>{state().node || `least loaded${chosenNode() ? ` (now ${chosenNode().name})` : ''}`}</dd>
                <dt>Limits</dt><dd class="mono">{state().memory_gib} GiB · {state().cpu} CPU · {state().pids} PIDs{Number(state().gpu_count) > 0 ? ` · ${state().gpu_count} GPU` : ''}</dd>
                <dt>Egress mode</dt><dd>{EGRESS_MODES.find((m) => m.value === state().egress_mode)?.label}</dd>
              </Show>
              <dt>Models</dt><dd>{state().models.length > 0 ? state().models.join(', ') : 'none'}</dd>
              <dt>Policy</dt><dd>{policies().find((p) => p.id === state().policy_id)?.name || 'default'}</dd>
              <dt>Mounts</dt>
              <dd>
                {state().mounts.length > 0
                  ? state().mounts.map((m) => `${m.mount_path}${m.read_only ? ' (ro)' : ''}`).join(', ')
                  : 'none'}
              </dd>
            </dl>
            <h3 class="eyebrow">Calls that will be made, in order</h3>
            <ol class="cw-calls">
              <li>{createCall(state()).label}<Show when={state().source !== 'recipe'}> <code class="mono">POST /sandboxes</code></Show></li>
              <For each={followUps(state(), '<new sandbox>', grantByName())}>{(c) => <li>{c.label}</li>}</For>
            </ol>
            <Show when={state().source !== 'recipe'}>
              <details class="small">
                <summary>Request body</summary>
                <pre class="mono">{JSON.stringify(bodyFor(state()), null, 2)}</pre>
              </details>
            </Show>
            <Show when={error()}>
              <p class="warn-note" role="alert">{error()}</p>
            </Show>
          </Show>
        </Wizard>
        <Show when={props.onCancel && !busy()}>
          <button type="button" class="ghost cw-cancel" onClick={() => props.onCancel()}>Cancel</button>
        </Show>
      </Show>
    </div>
  );
}
