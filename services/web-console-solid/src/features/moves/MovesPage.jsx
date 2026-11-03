// Moves between nodes, shown as tasks: nine steps per move, a drawer with the log and snapshots.
import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import PageHeader from '../shell/PageHeader.jsx';
import Breadcrumb from '../shell/Breadcrumb.jsx';
import HelpTip from '../../components/HelpTip.jsx';
import Drawer from '../../components/Drawer.jsx';
import SelectTable from '../../components/SelectTable.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import { usePolling } from '../../components/Poll.js';
import * as api from '../../api/moves.js';
import * as nodeApi from '../../api/nodes.js';
import * as sandboxApi from '../../api/sandboxes.js';
import MoveDialog from '../workbox/MoveDialog.jsx';
import { STEPS, STEP_LABELS, applyMove, emptySteps, formatBytes } from '../workbox/move.js';
import { useUi } from '../../state/ui.jsx';
import '../../styles/jobs.css';

// The nine steps of a move as a state map; a finished move shows every step done.
function stepStates(move) {
  if (move.state === 'done') return Object.fromEntries(STEPS.map((s) => [s, 'done']));
  return applyMove(emptySteps(), move);
}

// A compact strip of nine squares, one per step, with a text alternative.
function Strip(props) {
  const states = () => stepStates(props.move);
  const doneCount = () => STEPS.filter((s) => states()[s] === 'done').length;
  return (
    <span class="move-strip" role="img" aria-label={`${doneCount()} of ${STEPS.length} steps done`}>
      <For each={STEPS}>{(step) => <i class={states()[step]} title={`${STEP_LABELS[step]}: ${states()[step]}`} />}</For>
    </span>
  );
}

const MARKS = { pending: '○', running: '◐', done: '✓', failed: '✕' };

// A snapshot entry may be a string or an object; show something readable either way.
function snapshotText(entry) {
  if (typeof entry === 'string') return entry;
  return entry?.name || entry?.path || entry?.id || JSON.stringify(entry);
}

export default function MovesPage() {
  const [moves, setMoves] = createSignal([]);
  const [nodes, setNodes] = createSignal([]);
  const [boxes, setBoxes] = createSignal([]);
  const [error, setError] = createSignal('');
  const [openId, setOpenId] = createSignal(null);
  const [picker, setPicker] = createSignal(null); // 'move' | 'recover' | null
  const [chosen, setChosen] = createSignal('');
  const [target, setTarget] = createSignal('');
  const [moveBox, setMoveBox] = createSignal(null);
  const [busy, setBusy] = createSignal(false);
  const ui = useUi();

  const poll = usePolling(() => api.list(), {
    everyMs: 3000,
    onData: (data) => { setMoves(data.items || []); setError(''); },
    onError: (failure) => setError(failure.message),
  });
  onMount(async () => {
    try { setNodes((await nodeApi.list()).items || []); } catch { /* the picker shows no nodes */ }
    try { setBoxes((await sandboxApi.list({ limit: 200 })).items || []); } catch { /* the picker shows no sandboxes */ }
  });

  const boxId = (item) => item.sandbox?.id ?? item.id;
  const boxName = (item) => item.sandbox?.name ?? item.name;
  const boxNode = (item) => item.node_name ?? item.sandbox?.node_name;
  const selectedBox = () => boxes().find((b) => boxId(b) === chosen());
  const open = () => moves().find((m) => m.id === openId());
  const recoverTargets = () => nodes().filter((n) => n.status === 'online' && n.name !== boxNode(selectedBox() || {}));
  const offlineBoxes = () => boxes().filter((b) => {
    const node = nodes().find((n) => n.name === boxNode(b));
    return node && node.status !== 'online';
  });

  function closePicker() { setPicker(null); setChosen(''); setTarget(''); }

  async function recover() {
    setBusy(true);
    try {
      const started = await api.recover(chosen(), target());
      closePicker();
      setOpenId(started.id);
      poll.refresh();
    } catch (failure) {
      ui.toast.show({ tone: 'bad', text: failure.message, undo: null, ms: 6000 });
    } finally {
      setBusy(false);
    }
  }

  const columns = [
    { key: 'sandbox_name', label: 'Sandbox', sortable: true, render: (m) => <strong>{m.sandbox_name}</strong> },
    { key: 'route', label: 'From → to', render: (m) => <span class="mono small">{m.from_node} → {m.to_node}</span> },
    { key: 'state', label: 'State', sortable: true, render: (m) => <StatusBadge status={m.state === 'done' ? 'succeeded' : m.state} label={m.state} /> },
    { key: 'step', label: 'Steps', render: (m) => <><Strip move={m} /> <span class="small muted">{STEP_LABELS[m.step] || m.step}</span></> },
    { key: 'bytes_moved', label: 'Sent', align: 'right', render: (m) => <span class="mono small">{formatBytes(m.bytes_moved)}</span> },
    { key: 'started_at', label: 'Started', sortable: true, render: (m) => <span class="small">{m.started_at ? new Date(m.started_at).toLocaleString() : '—'}</span> },
  ];

  return (
    <section class="page moves-page">
      <Breadcrumb items={[{ label: 'Operate' }, { label: 'Moves' }]} />
      <PageHeader
        title="Moves between nodes"
        subtitle="A move takes the sandbox with its folders and the agent's login to another node. The old node keeps a cold copy."
        actions={
          <>
            <button class="primary" onClick={() => setPicker('move')}>Move a sandbox…</button>
            <button class="secondary" onClick={() => setPicker('recover')}>Recover…</button>
            <HelpTip anchor="moves" />
          </>
        }
      />
      <Show when={error()}><p class="error">{error()}</p></Show>
      <SelectTable
        columns={columns}
        rows={moves()}
        rowKey="id"
        onRowClick={(m) => setOpenId(m.id)}
        empty={{ title: 'No moves yet', hint: 'Use "Move a sandbox…" to send one to another node.' }}
      />

      <Drawer open={!!open()} title={open() ? `${open().sandbox_name}: ${open().from_node} → ${open().to_node}` : ''} subtitle={open()?.state} onClose={() => setOpenId(null)}>
        <Show when={open()}>
          {(move) => (
            <div class="job-drawer">
              <Show when={move().error}><p class="error">{move().error}</p></Show>
              <ol class="move-steps">
                <For each={STEPS}>{(step) => (
                  <li><span aria-hidden="true">{MARKS[stepStates(move())[step]]}</span><span>{STEP_LABELS[step]} <span class="muted small">({stepStates(move())[step]})</span></span></li>
                )}</For>
              </ol>
              <h3 class="eyebrow">Log</h3>
              <Show when={(move().log || []).length > 0} fallback={<p class="muted small">No log entries.</p>}>
                <ol class="small"><For each={move().log}>{(entry) => <li><strong>{entry.step}</strong> {entry.detail}</li>}</For></ol>
              </Show>
              <h3 class="eyebrow">Snapshots</h3>
              <Show when={(move().snapshots || []).length > 0} fallback={<p class="muted small">No snapshots recorded.</p>}>
                <ul class="small mono"><For each={move().snapshots}>{(s) => <li>{snapshotText(s)}</li>}</For></ul>
              </Show>
            </div>
          )}
        </Show>
      </Drawer>

      <Drawer open={picker() !== null} title={picker() === 'recover' ? 'Recover a sandbox' : 'Move a sandbox'} subtitle={picker() === 'recover' ? 'For a sandbox whose node is gone. Work since its last scheduled snapshot is lost.' : 'Pick the sandbox; the next step checks the target before anything stops.'} onClose={closePicker}>
        <div class="move-picker">
          <label class="field"><span>Sandbox</span>
            <select value={chosen()} onChange={(e) => { setChosen(e.currentTarget.value); setTarget(''); }}>
              <option value="">Choose …</option>
              <For each={picker() === 'recover' ? offlineBoxes() : boxes()}>{(b) => <option value={boxId(b)}>{boxName(b)} (on {boxNode(b) || '?'})</option>}</For>
            </select>
          </label>
          <Show when={picker() === 'recover'}>
            <label class="field"><span>Recover on node</span>
              <select value={target()} onChange={(e) => setTarget(e.currentTarget.value)}>
                <option value="">Choose …</option>
                <For each={recoverTargets()}>{(n) => <option value={n.name}>{n.display_name || n.name}</option>}</For>
              </select>
            </label>
            <Show when={offlineBoxes().length === 0}><p class="muted small">No sandbox sits on an offline node.</p></Show>
            <button class="primary" disabled={busy() || !chosen() || !target()} onClick={recover}>Recover on that node</button>
          </Show>
          <Show when={picker() === 'move'}>
            <button class="primary" disabled={!chosen()} onClick={() => { setMoveBox(selectedBox()); closePicker(); }}>Continue</button>
          </Show>
        </div>
      </Drawer>

      <Show when={moveBox()}>
        <MoveDialog
          open={true}
          sandboxId={boxId(moveBox())}
          sandboxName={boxName(moveBox())}
          currentNode={boxNode(moveBox())}
          onClose={() => setMoveBox(null)}
          onMoved={() => { setMoveBox(null); poll.refresh(); }}
        />
      </Show>
    </section>
  );
}
