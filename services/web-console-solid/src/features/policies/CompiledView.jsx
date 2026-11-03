// The compiled preview: YAML, skipped rules, hot and cold sections, and the diff.
// Props: preview (answer of POST /policies/preview or null), loading, error.

import { For, Show } from 'solid-js';

// Shows exactly what the runtime will receive, before anything is saved.
export default function CompiledView(props) {
  const p = () => props.preview;
  return (
    <div class="pw-compiled">
      <Show when={props.loading}><p class="muted">Loading…</p></Show>
      <Show when={props.error}><p class="error">{props.error}</p></Show>
      <Show when={p()}>
        <p>
          <span class={`badge ${p().valid ? 'ok' : 'bad'}`}><i />{p().valid ? 'valid' : 'invalid'}</span>{' '}
          <For each={p().hot || []}>{(s) => <span class="badge ok"><i />{s} · applies now</span>}</For>{' '}
          <For each={p().needs_recreate || []}>{(s) => <span class="badge warn"><i />{s} · needs recreate</span>}</For>
        </p>
        <Show when={(p().errors || []).length > 0}>
          <ul class="pw-errors">
            <For each={p().errors}>{(e) => <li><span class="mono">{e.path}</span> {e.message}</li>}</For>
          </ul>
        </Show>
        <Show when={(p().skipped || []).length > 0}>
          <h4>Not expressible in the runtime</h4>
          <ul>
            <For each={p().skipped}>{(s) => <li><span class="mono">{s.rule}</span> <span class="muted small">{s.why}</span></li>}</For>
          </ul>
        </Show>
        <h4>Compiled OpenShell document</h4>
        <pre class="mono pw-yaml">{p().compiled_yaml || '(empty)'}</pre>
        <Show when={p().diff}>
          <h4>Change against the current document</h4>
          <pre class="mono pw-yaml pw-diff">
            <For each={String(p().diff).split('\n')}>
              {(line) => <span classList={{ add: line.startsWith('+') && !line.startsWith('+++'), del: line.startsWith('-') && !line.startsWith('---') }}>{line + '\n'}</span>}
            </For>
          </pre>
        </Show>
      </Show>
    </div>
  );
}
