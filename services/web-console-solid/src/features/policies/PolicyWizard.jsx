// The policy wizard: Network, DNS, Filesystem, Process and Landlock, Tools, Observation,
// Review. Nothing is written before Finish; Review shows the compiled document first.
// Props: policyId (edit), sandboxId (suggestions, assign, observation), onClose().

import { For, Show, createEffect, createMemo, createSignal, on, onMount } from 'solid-js';
import Wizard from '../../components/Wizard.jsx';
import { navigate, paths } from '../../routing.js';
import { useUi } from '../../state/ui.jsx';
import * as policiesApi from '../../api/policies.js';
import * as trafficApi from '../../api/traffic.js';
import * as mcpApi from '../../api/mcp.js';
import MountEditor from '../sandboxes/MountEditor.jsx';
import NetworkStep from './NetworkStep.jsx';
import DnsStep from './DnsStep.jsx';
import ProcessStep from './ProcessStep.jsx';
import ObservationStep from './ObservationStep.jsx';
import ReviewStep from './ReviewStep.jsx';
import { dnsToPayload } from './policyForms.js';
import {
  applyPlan, dnsSuggestionsFrom, filesystemLists, newState, runPlan, setFilesystemLists,
  stepsFor, suggestionsFrom, validateStep,
} from './policyWizard.js';
import '../../styles/policy-wizard.css';

// The calls the plan can make, bound to the real API.
const OPS = {
  create: (call) => policiesApi.create(call.body),
  update: (call) => policiesApi.update(call.id, call.body),
  dnsRule: (call) => policiesApi.createDnsRule(dnsToPayload(call.form)),
  assign: (call) => policiesApi.assignPolicy(call.sandboxId, call.policyId),
  egress: (call) => policiesApi.setEgressMode(call.sandboxId, call.mode),
  window: (call) => policiesApi.openWindow(call.sandboxId, call.minutes, 'policy wizard evaluation window', 'audit'),
};

// Filesystem step: F15's MountEditor in policy mode edits the read-only and read-write lists
// with the same spec-in, spec-out contract.
function FilesystemStep(props) {
  return (
    <div class="pw-step">
      <h2>Filesystem</h2>
      <p class="warn-note">This section applies after a recreate of the sandbox (ADR 0008).</p>
      <MountEditor policyMode spec={props.spec} onChange={props.onSpec} />
    </div>
  );
}

// Tools step: what is attached to the sandbox, read-only, with a link to change it.
function ToolsStep(props) {
  const [servers, setServers] = createSignal(null);
  const [error, setError] = createSignal(null);
  onMount(async () => {
    try {
      const answer = await mcpApi.attached(props.sandboxId);
      setServers(answer.items || answer.servers || []);
    } catch (failure) {
      setError(failure.message);
    }
  });
  return (
    <div class="pw-step">
      <h2>Tools and MCP</h2>
      <p class="muted small">MCP servers attached to this sandbox. They are managed on the MCP page; this step only shows them.</p>
      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={servers() === null && !error()}><p class="muted">Loading…</p></Show>
      <Show when={servers()}>
        <Show when={servers().length > 0} fallback={<p class="notice">No MCP server is attached.</p>}>
          <ul class="pw-suggest">
            <For each={servers()}>{(s) => <li><span class="mono">{s.server_key || s.key || s.name}</span></li>}</For>
          </ul>
        </Show>
      </Show>
      <button type="button" class="secondary" onClick={() => navigate(paths.menu('mcp'))}>Open the MCP page</button>
    </div>
  );
}

// Loads the policy (when editing), walks the steps and applies the plan.
export default function PolicyWizard(props) {
  const ui = useUi();
  const [state, setState] = createSignal(newState({ sandboxId: props.sandboxId || null }));
  const [step, setStep] = createSignal('network');
  const [loadError, setLoadError] = createSignal(null);
  const [decisions, setDecisions] = createSignal([]);
  const [preview, setPreview] = createSignal(null);
  const [previewError, setPreviewError] = createSignal(null);
  const [previewing, setPreviewing] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [results, setResults] = createSignal(null);

  const patch = (fields) => setState((current) => ({ ...current, ...fields }));
  const steps = createMemo(() => stepsFor(state().sandboxId));
  const plan = createMemo(() => applyPlan(state()));
  const networkSuggestions = createMemo(() => suggestionsFrom(decisions(), state().spec));
  const dnsSuggestions = createMemo(() => dnsSuggestionsFrom(decisions(), state().dns));

  onMount(async () => {
    try {
      if (props.policyId) {
        const policy = await policiesApi.get(props.policyId);
        setState(newState({ policy: policy.policy || policy, sandboxId: props.sandboxId || null }));
      }
      if (props.sandboxId) {
        const answer = await trafficApi.decisions({ sandbox: props.sandboxId, window: '24h' });
        setDecisions(answer.items || []);
      }
    } catch (failure) {
      setLoadError(failure.message);
    }
  });

  async function refreshPreview() {
    setPreviewing(true);
    setPreviewError(null);
    try {
      setPreview(await policiesApi.preview({ spec: state().spec, sandbox_id: state().sandboxId }));
    } catch (failure) {
      setPreview(null);
      setPreviewError(failure.message);
    } finally {
      setPreviewing(false);
    }
  }

  // The preview is called on every entry to Review, so it never shows an old document.
  createEffect(on(step, (id) => { if (id === 'review') refreshPreview(); }, { defer: true }));

  const canNext = () => {
    const id = step();
    if (id === 'review') return !validateStep('review', state().spec, state()) && (preview()?.valid ?? true);
    return !validateStep(id, state().spec, state());
  };
  const problem = () => validateStep(step(), state().spec, state());

  async function finish() {
    setBusy(true);
    setResults(null);
    const run = await runPlan(plan(), OPS);
    setResults(run.results);
    setBusy(false);
    if (run.failed) {
      ui.toast.show({ tone: 'bad', text: 'The policy was not fully applied. See the failed call under Review.', undo: null, ms: 8000 });
      return;
    }
    const saved = run.results[0]?.value;
    const outcome = saved?.outcome;
    const note = outcome === 'requires_recreate'
      ? 'Policy saved. Filesystem, Landlock or process changes need a recreate.'
      : 'Policy saved and applied.';
    ui.toast.show({ tone: 'ok', text: note, undo: null, ms: 6000 });
    navigate(`/policies?id=${run.policyId}`);
    props.onClose?.();
  }

  return (
    <section class="page policy-wizard">
      <header class="page-head">
        <div>
          <h1>{props.policyId ? 'Edit policy' : 'New policy'}</h1>
          <p class="muted">Write the policy step by step. Nothing is saved until you apply it on the last step.</p>
        </div>
        <button type="button" class="secondary" onClick={() => props.onClose?.()}>Cancel</button>
      </header>
      <Show when={loadError()}><p class="error">{loadError()}</p></Show>
      <Wizard
        steps={steps()}
        step={step()}
        onStep={setStep}
        canNext={canNext}
        onFinish={finish}
        finishLabel={props.policyId ? 'Save and apply' : 'Create and apply'}
        busy={busy()}
      >
        <Show when={problem()}><p class="warn-note" role="alert">{problem()}</p></Show>
        <Show when={step() === 'network'}>
          <NetworkStep spec={state().spec} onSpec={(spec) => patch({ spec })} suggestions={networkSuggestions()} hasSandbox={!!state().sandboxId} />
        </Show>
        <Show when={step() === 'dns'}>
          <DnsStep dns={state().dns} onDns={(dns) => patch({ dns })} suggestions={dnsSuggestions()} />
        </Show>
        <Show when={step() === 'filesystem'}>
          <FilesystemStep spec={state().spec} onSpec={(spec) => patch({ spec })} />
        </Show>
        <Show when={step() === 'process'}>
          <ProcessStep spec={state().spec} onSpec={(spec) => patch({ spec })} />
        </Show>
        <Show when={step() === 'tools'}>
          <ToolsStep sandboxId={state().sandboxId} />
        </Show>
        <Show when={step() === 'observation'}>
          <ObservationStep
            sandboxId={state().sandboxId}
            egressMode={state().egressMode}
            minutes={state().windowMinutes}
            onMode={(egressMode) => patch({ egressMode })}
            onMinutes={(windowMinutes) => patch({ windowMinutes })}
          />
        </Show>
        <Show when={step() === 'review'}>
          <ReviewStep
            state={state()}
            onName={(name) => patch({ name })}
            onScope={(scope) => patch({ scope })}
            plan={plan()}
            preview={preview()}
            loading={previewing()}
            error={previewError()}
            results={results()}
            onRefresh={refreshPreview}
          />
        </Show>
      </Wizard>
    </section>
  );
}
