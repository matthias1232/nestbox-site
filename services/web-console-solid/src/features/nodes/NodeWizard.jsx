// Add a node: one wizard, three sources (script, connect by IP, rent), an optional model.
// Nothing is written before Finish; Finish makes one call, `POST /nodes/quick`, and the page
// then follows the job. Props: onClose(), onDone() (called when the job ends well).
import { For, Show, createMemo, createSignal, onMount } from 'solid-js';
import Wizard from '../../components/Wizard.jsx';
import * as hosting from '../../api/hosting.js';
import * as models from '../../api/models.js';
import * as nodeOps from '../../api/nodeOps.js';
import * as tenancy from '../../api/tenancy.js';
import JobProgress from './JobProgress.jsx';
import { NAME_LIMIT, SOURCES, blankState, bodyFor, deployableModels, firstProblem, reviewLines, steps, validate } from './nodeWizard.js';

const money = (milli) => (milli ? `${(milli / 1000).toFixed(2)}/h` : '');

export default function NodeWizard(props) {
  const [state, setState] = createSignal(blankState());
  const [step, setStep] = createSignal('source');
  const [sites, setSites] = createSignal([]);
  const [accounts, setAccounts] = createSignal([]);
  const [types, setTypes] = createSignal([]);
  const [catalog, setCatalog] = createSignal([]);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal(null);
  const [started, setStarted] = createSignal(null);

  const patch = (change) => setState({ ...state(), ...change });
  const patchIn = (key, change) => setState({ ...state(), [key]: { ...state()[key], ...change } });
  const stepList = createMemo(() => steps(state().source));
  const problem = () => validate(step(), state());
  const chosenType = () => types().find((t) => t.name === state().rent.hostType);

  onMount(async () => {
    try {
      const list = (await tenancy.sites()).items || [];
      setSites(list);
      if (list.length > 0) patch({ siteId: list[0].id });
    } catch (failure) { setError(failure.message); }
    // Renting and models are optional extras: a failure there must not block adding by script.
    try { setAccounts((await hosting.accounts()).items || []); } catch { setAccounts([]); }
    try {
      const answer = await models.catalog();
      setCatalog(deployableModels(answer.items || answer.models || []));
    } catch { setCatalog([]); }
  });

  async function pickAccount(id) {
    patchIn('rent', { accountId: id, hostType: '', location: '' });
    setTypes([]);
    if (!id) return;
    try { setTypes((await hosting.types(id)).items || []); } catch (failure) { setError(failure.message); }
  }

  const names = () => ({
    site: sites().find((s) => s.id === state().siteId)?.name,
    account: accounts().find((a) => a.id === state().rent.accountId)?.name,
    model: catalog().find((m) => m.key === state().modelKey)?.name,
  });

  async function finish() {
    const found = firstProblem(state());
    if (found) { setStep(found.step); setError(found.message); return; }
    setBusy(true);
    setError(null);
    try {
      const answer = await nodeOps.quick(bodyFor(state()));
      setStarted({ jobId: answer.job_id, joinCommand: answer.join_command || null });
    } catch (failure) {
      setError(failure.message);
    } finally {
      // A credential leaves the page as soon as the request is answered.
      patchIn('connect', { password: '', privateKey: '' });
      setBusy(false);
    }
  }

  return (
    <section class="page">
      <header class="page-head">
        <div><h1>Add node</h1><p class="muted">Three ways in, one wizard. Nothing is created before the last step.</p></div>
        <div class="controls"><button class="secondary" onClick={() => props.onClose?.()}>Close</button></div>
      </header>

      <Show when={!started()} fallback={
        <div class="card">
          <h2>Adding {state().name}</h2>
          <JobProgress jobId={started().jobId} joinCommand={started().joinCommand}
                       withModel={Boolean(state().modelKey)} onFinished={(job) => { if (job.status !== 'failed') props.onDone?.(); }} />
        </div>
      }>
        <Wizard steps={stepList()} step={step()} onStep={(id) => { setError(null); setStep(id); }}
                canNext={() => !problem()} onFinish={finish} busy={busy()}
                finishLabel={`Add node ${state().name.trim()}`}>
          <Show when={step() === 'source'}>
            <div class="form-grid" role="radiogroup" aria-label="Source">
              <For each={SOURCES}>
                {(source) => (
                  <label class="draft-card" classList={{ selected: state().source === source.id }}>
                    <input type="radio" name="node-source" checked={state().source === source.id}
                           disabled={source.id === 'rent' && accounts().length === 0}
                           onChange={() => patch({ source: source.id })} />{' '}
                    <strong>{source.title}</strong>
                    <p class="muted small">{source.text}</p>
                    <Show when={source.id === 'rent' && accounts().length === 0}>
                      <p class="muted small">Add a provider account under Rented machines first.</p>
                    </Show>
                  </label>
                )}
              </For>
            </div>
          </Show>

          <Show when={step() === 'placement'}>
            <div class="form-grid">
              <label class="field"><span>Site</span>
                <select value={state().siteId} onChange={(e) => patch({ siteId: e.currentTarget.value })}>
                  <For each={sites()}>{(s) => <option value={s.id}>{s.name} ({s.kind})</option>}</For>
                </select>
                <small class="muted">The node joins the active data center.</small>
              </label>
              <label class="field"><span>Node name</span>
                <input maxlength={NAME_LIMIT} value={state().name} placeholder="gpu-21"
                       onInput={(e) => patch({ name: e.currentTarget.value })} />
                <small class="muted">Lowercase letters, digits and dashes; at most {NAME_LIMIT} characters (OpenShell limit).</small>
              </label>
            </div>
          </Show>

          <Show when={step() === 'details' && state().source === 'connect'}>
            <div class="form-grid">
              <label class="field"><span>Address</span>
                <input value={state().connect.address} placeholder="203.0.113.10"
                       onInput={(e) => patchIn('connect', { address: e.currentTarget.value })} />
              </label>
              <label class="field"><span>SSH user</span>
                <input value={state().connect.sshUser} onInput={(e) => patchIn('connect', { sshUser: e.currentTarget.value })} />
              </label>
              <label class="field"><span>SSH port</span>
                <input type="number" min="1" max="65535" value={state().connect.sshPort}
                       onInput={(e) => patchIn('connect', { sshPort: e.currentTarget.value })} />
              </label>
              <label class="field"><span>WireGuard port</span>
                <input type="number" min="1" max="65535" value={state().connect.wgPort}
                       onInput={(e) => patchIn('connect', { wgPort: e.currentTarget.value })} />
                <small class="muted">UDP, must be open from this gateway.</small>
              </label>
            </div>
            <div class="form-grid" role="radiogroup" aria-label="Sign in with">
              <label class="draft-card"><input type="radio" name="wiz-cred" checked={state().connect.credential === 'password'}
                onChange={() => patchIn('connect', { credential: 'password' })} /> Password</label>
              <label class="draft-card"><input type="radio" name="wiz-cred" checked={state().connect.credential === 'key'}
                onChange={() => patchIn('connect', { credential: 'key' })} /> Private key</label>
            </div>
            <Show when={state().connect.credential === 'password'} fallback={
              <label class="field"><span>Private key</span>
                <textarea rows="6" class="mono" value={state().connect.privateKey} placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
                          onInput={(e) => patchIn('connect', { privateKey: e.currentTarget.value })} />
              </label>
            }>
              <label class="field"><span>Password</span>
                <input type="password" autocomplete="off" value={state().connect.password}
                       onInput={(e) => patchIn('connect', { password: e.currentTarget.value })} />
              </label>
            </Show>
            <p class="muted small">Used once to install the node, never stored.</p>
            <div class="form-grid" role="radiogroup" aria-label="Traffic from this node">
              <label class="draft-card"><input type="radio" name="wiz-lock" checked={state().connect.lock === 'strict'}
                onChange={() => patchIn('connect', { lock: 'strict' })} /> Strict: everything goes through this gateway (recommended)</label>
              <label class="draft-card"><input type="radio" name="wiz-lock" checked={state().connect.lock === 'exit_local'}
                onChange={() => patchIn('connect', { lock: 'exit_local' })} /> Exit locally: decisions here, traffic leaves at the node</label>
            </div>
          </Show>

          <Show when={step() === 'details' && state().source === 'rent'}>
            <div class="form-grid">
              <label class="field"><span>Account</span>
                <select value={state().rent.accountId} onChange={(e) => pickAccount(e.currentTarget.value)}>
                  <option value="">Choose…</option>
                  <For each={accounts()}>{(a) => <option value={a.id}>{a.name} ({a.kind})</option>}</For>
                </select>
              </label>
              <label class="field"><span>Machine type</span>
                <select value={state().rent.hostType} onChange={(e) => patchIn('rent', { hostType: e.currentTarget.value, location: '' })}>
                  <option value="">Choose…</option>
                  <For each={types()}>
                    {(t) => <option value={t.name}>{t.name} · {t.cores} cores, {Math.round(t.memory_mb / 1024)} GiB
                      {t.hourly_milli ? ` · ${money(t.hourly_milli)}` : ' · no published price'}</option>}
                  </For>
                </select>
              </label>
              <Show when={chosenType()?.locations?.length > 0}>
                <label class="field"><span>Location</span>
                  <select value={state().rent.location} onChange={(e) => patchIn('rent', { location: e.currentTarget.value })}>
                    <option value="">Provider default</option>
                    <For each={chosenType().locations}>{(l) => <option value={l}>{l}</option>}</For>
                  </select>
                </label>
              </Show>
              <label class="field"><span>Hand back after (minutes)</span>
                <input type="number" min="15" max="10080" value={state().rent.ttlMinutes}
                       onInput={(e) => patchIn('rent', { ttlMinutes: e.currentTarget.value })} />
              </label>
              <label class="field"><span>…or when empty for (minutes, 0 = never)</span>
                <input type="number" min="0" max="10080" value={state().rent.idleMinutes}
                       onInput={(e) => patchIn('rent', { idleMinutes: e.currentTarget.value })} />
              </label>
            </div>
            <p class="muted small">
              Budget line: up to {Math.round(Number(state().rent.ttlMinutes) / 60 * 100) / 100} h
              {chosenType()?.hourly_milli ? ` × ${money(chosenType().hourly_milli)} = ${((chosenType().hourly_milli * Number(state().rent.ttlMinutes)) / 60000).toFixed(2)}` : ''}.
            </p>
          </Show>

          <Show when={step() === 'model'}>
            <label class="field"><span>Deploy a model when the node is online (optional)</span>
              <select value={state().modelKey} onChange={(e) => patch({ modelKey: e.currentTarget.value })}>
                <option value="">None</option>
                <For each={catalog()}>{(m) => <option value={m.key}>{m.name || m.key}</option>}</For>
              </select>
              <small class="muted">Whether it fits is checked after the probe, not now.</small>
            </label>
          </Show>

          <Show when={step() === 'review'}>
            <ul>
              <For each={reviewLines(state(), names())}>{(line) => <li>{line}</li>}</For>
            </ul>
            <p class="muted small">The call this makes:</p>
            <pre class="mono small">POST /api/v1/nodes/quick{'\n'}{JSON.stringify(bodyFor(state(), true), null, 2)}</pre>
          </Show>

          <Show when={error()}><p class="error">{error()}</p></Show>
          <Show when={problem() && step() !== 'source'}><p class="muted small">{problem()}</p></Show>
        </Wizard>
      </Show>
    </section>
  );
}
