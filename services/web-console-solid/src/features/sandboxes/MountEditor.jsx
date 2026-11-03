// What this sandbox can touch: mounted roots (RO/RW), policy paths, roots that could be mounted.
// Props: sandboxId, or (policyMode) spec + onChange(spec) which edits only the policy paths.
import { For, Show, createResource, createSignal } from 'solid-js';
import * as mountsApi from '../../api/mounts.js';
import { navigate } from '../../routing.js';
import { useUi } from '../../state/ui.jsx';
import Banner from '../../components/Banner.jsx';
import MountRow from './MountRow.jsx';
import PathList from './PathList.jsx';
import * as logic from './mountEditor.js';
import '../../styles/mounts.css';

function PolicyOnly(props) {
  const lists = () => logic.policyLists(props.spec);
  return (
    <section class="mount-editor">
      <PathList ro={lists().ro} rw={lists().rw} onChange={(ro, rw) => props.onChange(logic.setPolicyLists(props.spec, ro, rw))} />
    </section>
  );
}

function Full(props) {
  const ui = useUi();
  const [effective, { refetch }] = createResource(() => props.sandboxId, (id) => mountsApi.get(id));
  const [rows, setRows] = createSignal(null);
  const [original, setOriginal] = createSignal([]);
  const [lists, setLists] = createSignal(null);
  const [choice, setChoice] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal(null);
  const [answer, setAnswer] = createSignal(null);

  // Load server state into the working copy whenever it arrives.
  const data = () => {
    const value = effective();
    if (value && rows() === null) {
      const initial = logic.toRows(value);
      setRows(initial);
      setOriginal(initial);
      setLists(logic.policyLists({ filesystem_policy: value.filesystem }));
    }
    return value;
  };
  const current = () => rows() || [];
  const problems = () => logic.errors(current());
  const dirty = () => logic.changed(current(), original());
  const available = () => (data()?.available_roots || []).filter((root) => !current().some((row) => row.root_id === root.root_id));

  async function apply(mode) {
    if (mode === 'now' && !window.confirm('Apply now recreates the sandbox. Running work is interrupted; persistent roots stay, anything written elsewhere inside the container is lost.')) return;
    setBusy(true); setFailure(null);
    try {
      const result = await mountsApi.put(props.sandboxId, logic.bodyFor(current(), mode));
      const next = logic.toRows(result);
      setRows(next); setOriginal(next);
      setLists(logic.policyLists({ filesystem_policy: result.filesystem }));
      setAnswer(result);
      ui.toast.show({ tone: 'ok', text: mode === 'now' ? 'Mounts applied; the sandbox was recreated.' : 'Mounts saved; they apply at the next recreate.', undo: null, ms: 6000 });
      refetch();
    } catch (error) {
      setFailure(error.message + (error.code ? ` (${error.code})` : ''));
      ui.toast.show({ tone: 'bad', text: `Mounts not applied: ${error.message}`, undo: null, ms: 6000 });
    } finally {
      setBusy(false);
    }
  }

  const needsRecreate = () => (answer() || data())?.needs_recreate || dirty();

  return (
    <section class="mount-editor">
      <h2>What this sandbox can touch</h2>
      <Show when={effective.loading && !data()}><p class="muted">Loading…</p></Show>
      <Show when={effective.error}><p class="error">{effective.error.message}</p></Show>
      <Show when={data()}>
        <Show when={needsRecreate()}>
          <Banner tone="warn">Changes apply after a recreate; running work is interrupted.</Banner>
        </Show>
        <Show when={failure()}><p class="error">{failure()}</p></Show>
        <Show when={answer()}><p class="notice">Saved. {answer().needs_recreate ? 'A recreate is still pending.' : 'The sandbox runs with these mounts.'}</p></Show>

        <h3 class="eyebrow">Mounted roots</h3>
        <Show when={current().length} fallback={<p class="muted small">Nothing is mounted. The sandbox sees only its own container filesystem.</p>}>
          <div class="mount-table">
            <table>
              <thead><tr><th>Root</th><th>Owner</th><th>Node</th><th>Path</th><th>Access</th><th>Size</th><th /></tr></thead>
              <tbody>
                <For each={current()}>
                  {(row) => (
                    <MountRow
                      row={row}
                      error={problems()[row.root_id]}
                      onPath={(path) => setRows(logic.setPath(current(), row.root_id, path))}
                      onFlip={() => setRows(logic.flip(current(), row.root_id))}
                      onRemove={() => setRows(logic.remove(current(), row.root_id))}
                    />
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>

        <div class="mount-add">
          <select aria-label="Mount another root" value={choice()} onChange={(e) => setChoice(e.currentTarget.value)}>
            <option value="">Mount another…</option>
            <For each={available()}>{(root) => <option value={root.root_id}>{root.kind} · {root.slug}{root.owner ? ` (${root.owner})` : ''}</option>}</For>
          </select>
          <button
            type="button"
            class="secondary tiny"
            disabled={!choice()}
            onClick={() => {
              const root = available().find((item) => item.root_id === choice());
              if (root) setRows(logic.add(current(), root));
              setChoice('');
            }}
          >Mount</button>
          <a href="/persistent?new=1" onClick={(e) => { e.preventDefault(); navigate('/persistent?new=1'); }}>New folder…</a>
        </div>

        <h3 class="eyebrow">Filesystem policy</h3>
        <PathList ro={lists()?.ro || []} rw={lists()?.rw || []} source={data().filesystem?.source} readOnly onChange={() => {}} />

        <div class="mount-footer">
          <button type="button" class="primary" disabled={busy() || !dirty() || Object.keys(problems()).length > 0} onClick={() => apply('now')}>{busy() ? 'Applying…' : 'Apply now (recreate)'}</button>
          <button type="button" class="secondary" disabled={busy() || !dirty() || Object.keys(problems()).length > 0} onClick={() => apply('later')}>Apply later</button>
          <button type="button" class="ghost" disabled={busy() || !dirty()} onClick={() => setRows(original())}>Discard</button>
        </div>
      </Show>
    </section>
  );
}

export default function MountEditor(props) {
  return (
    <Show when={props.policyMode} fallback={<Full sandboxId={props.sandboxId} />}>
      <PolicyOnly spec={props.spec} onChange={(spec) => props.onChange && props.onChange(spec)} />
    </Show>
  );
}
