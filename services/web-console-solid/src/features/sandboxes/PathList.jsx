// The filesystem policy's read-only and read-write paths, two editable columns.
// Props: ro, rw (arrays), onChange(ro, rw), source ('policy' | 'runner_default' | undefined), readOnly?.
import { For, Show, createSignal } from 'solid-js';
import { navigate } from '../../routing.js';

function Column(props) {
  const [draft, setDraft] = createSignal('');
  const problem = () => {
    const value = draft().trim();
    if (!value) return null;
    if (!value.startsWith('/')) return 'A path must be absolute.';
    if (value.split('/').includes('..')) return 'A path must not contain "..".';
    if (props.paths.includes(value)) return 'Already listed.';
    return null;
  };
  function submit(e) {
    e.preventDefault();
    if (!draft().trim() || problem()) return;
    props.onChange([...props.paths, draft().trim()]);
    setDraft('');
  }
  return (
    <div class="path-column">
      <h3 class="eyebrow">{props.title}</h3>
      <ul class="path-list">
        <For each={props.paths} fallback={<li class="muted small">None.</li>}>
          {(path, index) => (
            <li>
              <span class="mono">{path}</span>
              <Show when={!props.readOnly}>
                <button type="button" class="tiny ghost" aria-label={`Remove ${path} from ${props.title}`} onClick={() => props.onChange(props.paths.filter((_, i) => i !== index()))}>Remove</button>
              </Show>
            </li>
          )}
        </For>
      </ul>
      <Show when={!props.readOnly}>
        <form class="path-add" onSubmit={submit}>
          <input class="mono" aria-label={`Add path to ${props.title}`} placeholder="/path" value={draft()} onInput={(e) => setDraft(e.currentTarget.value)} />
          <button type="submit" class="tiny secondary" disabled={!draft().trim() || !!problem()}>Add</button>
        </form>
        <Show when={problem()}><div class="small mount-error">{problem()}</div></Show>
      </Show>
    </div>
  );
}

export default function PathList(props) {
  return (
    <div class="path-lists">
      <div class="path-columns">
        <Column title="Read-only paths" paths={props.ro} readOnly={props.readOnly} onChange={(next) => props.onChange(next, props.rw)} />
        <Column title="Read-write paths" paths={props.rw} readOnly={props.readOnly} onChange={(next) => props.onChange(props.ro, next)} />
      </div>
      <Show when={props.source}>
        <p class="muted small">
          <Show
            when={props.source === 'runner_default'}
            fallback={'From the sandbox policy.'}
          >
            Runner default. Change it in the{' '}
            <a href="/policies" onClick={(e) => { e.preventDefault(); navigate('/policies'); }}>policy wizard</a>.
          </Show>
        </p>
      </Show>
    </div>
  );
}
