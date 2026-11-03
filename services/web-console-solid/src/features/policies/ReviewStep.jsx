// Wizard last step: name, scope, the compiled preview and the calls Apply will make.
// Props: state, onName(text), onScope(text), plan, preview, loading, error, results, onRefresh().

import { For, Show } from 'solid-js';
import CompiledView from './CompiledView.jsx';

// Nothing is written until the Finish button; this step shows everything that will be.
export default function ReviewStep(props) {
  return (
    <div class="pw-step">
      <h2>Review</h2>
      <div class="pw-add">
        <label class="pw-field"><span>Name</span>
          <input class="pf-input" value={props.state.name} onInput={(e) => props.onName(e.currentTarget.value)} />
        </label>
        <label class="pw-field"><span>Scope</span>
          <select class="pf-input" value={props.state.scope} disabled={!!props.state.policyId} onChange={(e) => props.onScope(e.currentTarget.value)}>
            <option value="sandbox">sandbox (assign it to sandboxes)</option>
            <option value="global">global override (replaces every sandbox policy)</option>
          </select>
        </label>
        <button type="button" class="secondary" onClick={props.onRefresh}>Refresh preview</button>
      </div>
      <Show when={props.state.scope === 'global'}>
        <p class="warn-note">A global policy replaces every sandbox policy and blocks sandbox-level edits.</p>
      </Show>

      <CompiledView preview={props.preview} loading={props.loading} error={props.error} />

      <h3>Calls Apply will make, in order</h3>
      <ol class="pw-plan">
        <For each={props.plan}>
          {(item, i) => {
            const result = () => props.results?.[i()];
            return (
              <li>
                {item.label}
                <Show when={result()}>
                  {' '}<span class={`badge ${result().ok ? 'ok' : 'bad'}`}><i />{result().ok ? 'done' : 'failed'}</span>
                  <Show when={!result().ok}> <span class="error small">{result().message}</span></Show>
                </Show>
              </li>
            );
          }}
        </For>
      </ol>
    </div>
  );
}
