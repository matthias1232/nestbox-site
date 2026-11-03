// The "New cluster" dialog: name, model, strategy and the members, written only on Create.
import { For, Show, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import * as clusters from '../../api/clusters.js';
import { STRATEGIES, modelsWithInstalls, pickableInstalls } from './clusterHealth.js';
import '../../styles/controls.css';
import '../../styles/clusters.css';

// Props: installs (every install row), onClose(), onCreated(cluster).
export default function NewClusterDialog(props) {
  const models = createMemo(() => modelsWithInstalls(props.installs).filter((m) => pickableInstalls(props.installs, m.key).length > 0));
  const [name, setName] = createSignal('');
  const [modelKey, setModelKey] = createSignal(models()[0]?.key || '');
  const [strategy, setStrategy] = createSignal('round_robin');
  const [picked, setPicked] = createSignal(new Set());
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal(null);
  let dialog;

  const choices = createMemo(() => pickableInstalls(props.installs, modelKey()));
  const valid = () => name().trim().length > 0 && modelKey() && picked().size > 0;

  const changeModel = (key) => { setModelKey(key); setPicked(new Set()); };
  const toggle = (id) => {
    const next = new Set(picked());
    if (next.has(id)) next.delete(id); else next.add(id);
    setPicked(next);
  };

  const onKey = (event) => {
    if (event.key === 'Escape') { props.onClose(); return; }
    if (event.key !== 'Tab' || !dialog) return;
    const items = [...dialog.querySelectorAll('button:not([disabled]), input:not([disabled]), select:not([disabled])')];
    if (items.length === 0) return;
    const firstItem = items[0];
    const lastItem = items[items.length - 1];
    if (event.shiftKey && document.activeElement === firstItem) { event.preventDefault(); lastItem.focus(); }
    else if (!event.shiftKey && document.activeElement === lastItem) { event.preventDefault(); firstItem.focus(); }
  };
  onMount(() => {
    window.addEventListener('keydown', onKey);
    const input = dialog && dialog.querySelector('input');
    if (input) input.focus();
  });
  onCleanup(() => window.removeEventListener('keydown', onKey));

  async function create() {
    setBusy(true);
    setError(null);
    try {
      const made = await clusters.create({
        name: name().trim(),
        model_key: modelKey(),
        strategy: strategy(),
        install_ids: [...picked()],
      });
      props.onCreated(made);
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <div class="nb-dialog-backdrop" onClick={() => props.onClose()} />
      <div class="nb-dialog cluster-dialog" role="dialog" aria-modal="true" aria-label="New cluster" ref={dialog}>
        <h3>New model cluster</h3>
        <Show when={models().length > 0} fallback={<p class="muted">No model has an installed copy yet. Install a model on a node first, then group its installs here.</p>}>
          <div class="nb-dialog-body cluster-form">
            <label class="cluster-field">
              <span class="eyebrow">Name</span>
              <input type="text" value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="deepseek-x4" />
            </label>
            <label class="cluster-field">
              <span class="eyebrow">Model</span>
              <select value={modelKey()} onChange={(e) => changeModel(e.currentTarget.value)}>
                <For each={models()}>{(m) => <option value={m.key}>{m.name}</option>}</For>
              </select>
            </label>
            <fieldset class="cluster-field">
              <legend class="eyebrow">Strategy</legend>
              <For each={STRATEGIES}>
                {(s) => (
                  <label class="cluster-radio">
                    <input type="radio" name="strategy" checked={strategy() === s.id} onChange={() => setStrategy(s.id)} />
                    <span><strong>{s.label}</strong> <span class="muted small">{s.hint}</span></span>
                  </label>
                )}
              </For>
            </fieldset>
            <fieldset class="cluster-field">
              <legend class="eyebrow">Members ({picked().size} selected)</legend>
              <For each={choices()}>
                {(row) => (
                  <label class="cluster-radio">
                    <input type="checkbox" checked={picked().has(row.id)} onChange={() => toggle(row.id)} />
                    <span>{row.node_name || row.node_id} <span class="muted small mono">{row.state}</span></span>
                  </label>
                )}
              </For>
            </fieldset>
            <Show when={error()}><p class="warn-note" role="alert">{error()}</p></Show>
          </div>
        </Show>
        <div class="nb-dialog-actions">
          <button type="button" class="secondary" onClick={() => props.onClose()}>Cancel</button>
          <button type="button" class="primary" disabled={!valid() || busy()} onClick={create}>
            {busy() ? 'Creating…' : 'Create cluster'}
          </button>
        </div>
      </div>
    </>
  );
}
