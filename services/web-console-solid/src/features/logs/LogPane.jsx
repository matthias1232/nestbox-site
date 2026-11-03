// One log pane: a header with source, follow, filter, problems-only, copy, pop-out and close,
// and the merged log text below. It polls its own source while Follow is on.
import { For, Show, createEffect, createSignal, on, onCleanup, onMount } from 'solid-js';
import LogView from '../../components/LogView.jsx';
import IconButton from '../../components/IconButton.jsx';
import { IconClose, IconCopy, IconPopOut } from '../../components/icons.jsx';
import { loaderFor, sourceLabel } from './logLoaders.js';

const POLL_MS = 3000;

// props: source, options (source lists), sandboxes, onSource(value), onClose, onPopOut, canClose
export default function LogPane(props) {
  const [text, setText] = createSignal('');
  const [error, setError] = createSignal(null);
  const [readAt, setReadAt] = createSignal(null);
  const [follow, setFollow] = createSignal(true);
  const [filter, setFilter] = createSignal('');
  const [problems, setProblems] = createSignal(false);
  let timer = null;
  let alive = true;

  async function read() {
    try {
      const load = loaderFor(props.source, { sandboxes: props.sandboxes });
      const result = await load();
      if (!alive) return;
      setText(result);
      setReadAt(new Date());
      setError(null);
    } catch (failure) {
      if (alive) setError(failure.message || 'The log could not be read.');
    }
  }

  function schedule() {
    if (timer) clearInterval(timer);
    timer = follow() ? setInterval(read, POLL_MS) : null;
  }

  onMount(() => { read(); schedule(); });
  onCleanup(() => { alive = false; if (timer) clearInterval(timer); });
  createEffect(on(() => props.source, () => { setText(''); read(); }, { defer: true }));
  createEffect(on(follow, schedule, { defer: true }));

  const label = () => sourceLabel(props.source, props.names);
  const copy = () => navigator.clipboard && navigator.clipboard.writeText(text());

  return (
    <section class="log-pane" aria-label={label()}>
      <header class="log-pane-head">
        <select aria-label="Log source" value={props.source} onChange={(e) => props.onSource(e.currentTarget.value)}>
          <option value="all">All running sandboxes ({props.sandboxes.length})</option>
          <For each={props.sandboxes}>{(item) => <option value={`sandbox:${item.id}`}>Sandbox {item.name}</option>}</For>
          <For each={props.nodes}>{(item) => <option value={`node:${item.id}`}>Node {item.name}</option>}</For>
          <For each={props.components}>{(name) => <option value={`system:${name}`}>Platform: {name}</option>}</For>
        </select>
        <button class={follow() ? 'primary tiny' : 'secondary tiny'} onClick={() => setFollow(!follow())}>
          {follow() ? 'Following' : 'Paused'}
        </button>
        <input type="search" aria-label="Filter lines" placeholder="Filter lines…" value={filter()}
               onInput={(e) => setFilter(e.currentTarget.value)} />
        <label class="small"><input type="checkbox" checked={problems()} onChange={(e) => setProblems(e.currentTarget.checked)} /> problems only</label>
        <span class="log-pane-tools">
          <IconButton icon={IconCopy} label="Copy visible log" onClick={copy} />
          <IconButton icon={IconPopOut} label="Open this pane in its own window" onClick={props.onPopOut}
                      disabled={!props.canPopOut} />
          <Show when={props.canClose}>
            <IconButton icon={IconClose} label="Close pane" onClick={props.onClose} />
          </Show>
        </span>
      </header>
      <Show when={error()}><p class="error small">{error()}</p></Show>
      <LogView text={text()} filter={filter()} onlyProblems={problems()} follow={follow()}
               empty="No log lines yet. Start or use a sandbox and its log appears here." />
      <footer class="muted small">{label()} · {readAt() ? `read ${readAt().toLocaleTimeString()}` : 'reading…'}</footer>
    </section>
  );
}
