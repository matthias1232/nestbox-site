// Delete a node: drains it first when something still runs on it.
// Props: node ({id, name, display_name}), nodes (all nodes, for the drain target), onClose(), onDeleted().
import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import * as nodeOps from '../../api/nodeOps.js';
import * as nodesApi from '../../api/nodes.js';
import Switch from '../../components/Switch.jsx';
import JobProgress from './JobProgress.jsx';

export default function DeleteNodeDialog(props) {
  const [summary, setSummary] = createSignal(null);
  const [target, setTarget] = createSignal('');
  const [stopOnly, setStopOnly] = createSignal(false);
  const [drainJob, setDrainJob] = createSignal(null);
  const [drained, setDrained] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal(null);
  let panel;

  const others = () => (props.nodes || []).filter((n) => n.id !== props.node.id && n.status !== 'offline');
  const sandboxes = () => summary()?.sandboxes?.total ?? 0;
  const installs = () => (summary()?.installs || []).length;
  const empty = () => Boolean(summary()?.empty) || drained();
  const canDrain = () => stopOnly() || Boolean(target());

  async function loadSummary() {
    try { setSummary(await nodeOps.summary(props.node.id)); } catch (failure) { setError(failure.message); }
  }

  onMount(() => {
    loadSummary();
    panel?.focus();
    // Escape closes; Tab stays inside the dialog.
    const onKey = (event) => {
      if (event.key === 'Escape') { props.onClose?.(); return; }
      if (event.key !== 'Tab' || !panel) return;
      const items = [...panel.querySelectorAll('button:not([disabled]), select, input, [tabindex="0"]')];
      if (items.length === 0) return;
      const first = items[0];
      const last = items[items.length - 1];
      if (event.shiftKey && (document.activeElement === first || document.activeElement === panel)) { event.preventDefault(); last.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });

  async function drain() {
    setBusy(true);
    setError(null);
    try {
      const answer = await nodeOps.drain(props.node.id, { to_node: stopOnly() ? null : target(), stop_only: stopOnly() });
      setDrainJob(answer.job_id);
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    setBusy(true);
    setError(null);
    try {
      await nodesApi.removeJoined(props.node.id);
      props.onDeleted?.();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <div class="nb-dialog-backdrop" onClick={() => props.onClose?.()} />
      <div class="nb-dialog" role="dialog" aria-modal="true" aria-label={`Delete node ${props.node.name}`} tabindex="-1" ref={panel}>
        <h3>Delete node {props.node.name}</h3>
        <div class="nb-dialog-body">
          <Show when={summary()} fallback={<p class="muted">Loading…</p>}>
            <Show when={!empty()} fallback={<p>Nothing runs on this node. It can be deleted now.</p>}>
              <p>
                This node runs {sandboxes()} sandbox{sandboxes() === 1 ? '' : 'es'} and holds {installs()} model
                install{installs() === 1 ? '' : 's'}. Drain first:
              </p>
              <Show when={!drainJob()}>
                <label class="field"><span>Move sandboxes to</span>
                  <select value={target()} disabled={stopOnly()} onChange={(e) => setTarget(e.currentTarget.value)}>
                    <option value="">Choose a node…</option>
                    <For each={others()}>{(n) => <option value={n.name}>{n.display_name || n.name}</option>}</For>
                  </select>
                </label>
                <Switch checked={stopOnly()} onChange={setStopOnly} label={stopOnly() ? 'Stop only: on' : 'Stop only: off'} />
                <p class="muted small">Model installs are not moved; they are removed with the node.</p>
              </Show>
              <Show when={drainJob()}>
                <JobProgress jobId={drainJob()} steps={false}
                             onFinished={(job) => { if (job.status !== 'failed') setDrained(true); else setError('The drain failed; see the task for details.'); }} />
              </Show>
            </Show>
          </Show>
          <Show when={error()}><p class="error">{error()}</p></Show>
        </div>
        <div class="nb-dialog-actions">
          <button type="button" class="secondary" onClick={() => props.onClose?.()}>Cancel</button>
          <Show when={summary() && !empty() && !drainJob()}>
            <button type="button" class="primary" disabled={busy() || !canDrain()} onClick={drain}>
              {stopOnly() ? 'Stop all sandboxes' : 'Drain to the chosen node'}
            </button>
          </Show>
          <button type="button" class="danger" disabled={busy() || !summary() || !empty()} onClick={remove}>
            Delete node {props.node.name}
          </button>
        </div>
      </div>
    </>
  );
}
