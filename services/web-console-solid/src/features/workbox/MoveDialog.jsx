// MoveDialog: pick another node, read the preflight, start the move, and follow its nine steps.
//
// The dialog never decides whether a move can work — it asks, shows the answer, and only offers
// "Move now" when the server said there is nothing in the way. Progress arrives on the state
// stream and is polled every five seconds as a fallback, because a stream that drops must not
// leave the user staring at a step that finished ten minutes ago.

import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js';

import * as moves from '../../api/moves.js';
import * as nodesApi from '../../api/nodes.js';
import ConfirmDialog from '../../components/ConfirmDialog.jsx';
import {
  STEP_LABELS,
  STEPS,
  applyEvent,
  applyMove,
  emptySteps,
  failureNote,
  formatBytes,
  orderNodes,
  progress,
  rolledBack,
} from './move.js';
import { onControl } from './useLive.js';

const MARKS = { pending: '○', running: '◐', done: '✓', failed: '✕' };

/** The mark in front of a step. */
function mark(state) {
  return MARKS[state] || MARKS.pending;
}

export default function MoveDialog(props) {
  const [phase, setPhase] = createSignal('choose');
  const [nodes, setNodes] = createSignal([]);
  const [nodesError, setNodesError] = createSignal(null);
  const [selected, setSelected] = createSignal(null);
  const [preflight, setPreflight] = createSignal(null);
  const [preflightError, setPreflightError] = createSignal(null);
  const [checking, setChecking] = createSignal(false);
  const [steps, setSteps] = createSignal(emptySteps());
  const [busy, setBusy] = createSignal(false);
  const [startError, setStartError] = createSignal(null);
  const [failure, setFailure] = createSignal(null);
  const [target, setTarget] = createSignal(null);

  let stopLive = null;
  let poll = null;

  function stopWatching() {
    if (stopLive) {
      stopLive();
      stopLive = null;
    }
    if (poll) {
      clearInterval(poll);
      poll = null;
    }
  }

  onCleanup(stopWatching);

  async function loadNodes() {
    try {
      const page = await nodesApi.list();
      setNodes(orderNodes(page.items || [], props.currentNode));
      setNodesError(null);
    } catch (failed) {
      setNodes([]);
      setNodesError(failed.message);
    }
  }

  function reset() {
    stopWatching();
    setPhase('choose');
    setNodes([]);
    setNodesError(null);
    setSelected(null);
    setPreflight(null);
    setPreflightError(null);
    setChecking(false);
    setSteps(emptySteps());
    setBusy(false);
    setStartError(null);
    setFailure(null);
    setTarget(null);
    void loadNodes();
  }

  // Every opening starts from nothing: a previous move's steps would read as this one's.
  createEffect(() => {
    if (props.open) reset();
    else stopWatching();
  });

  async function select(node) {
    const name = node.name;
    setSelected(name);
    setPreflight(null);
    setPreflightError(null);
    setChecking(true);
    try {
      const answer = await moves.preflight(props.sandboxId, name);
      if (selected() === name) setPreflight(answer);
    } catch (failed) {
      if (selected() === name) setPreflightError(failed.message);
    } finally {
      if (selected() === name) setChecking(false);
    }
  }

  const blockers = () => preflight()?.blockers || [];
  const canMove = () => Boolean(preflight()?.ok) && !blockers().length && !checking() && !busy();

  function onDetail(detail) {
    setSteps((current) => applyEvent(current, detail));
    if (detail.state === 'failed') {
      setPhase('failed');
      setFailure(failureNote(detail, props.currentNode));
      stopWatching();
      return;
    }
    // The move is done when the last step is done, not when any step is.
    if (detail.step === 'verify' && detail.state === 'done') {
      setPhase('done');
      stopWatching();
    }
  }

  async function refreshMove(id) {
    try {
      const view = await moves.get(id);
      setSteps((current) => applyMove(current, view));
      if (view.state === 'done') {
        setPhase('done');
        stopWatching();
      } else if (view.state === 'failed') {
        setPhase('failed');
        setFailure(
          failureNote(
            { message: view.error || 'The move failed.', rolled_back: rolledBack(view) },
            props.currentNode,
          ),
        );
        stopWatching();
      }
    } catch {
      // The stream is the primary source; a poll that fails is not a move that failed.
    }
  }

  function watch(id) {
    stopWatching();
    stopLive = onControl(
      () => props.sandboxId,
      (event) => {
        if (event.what !== 'move') return;
        onDetail(event.detail || {});
      },
    );
    poll = setInterval(() => {
      void refreshMove(id);
    }, 5000);
  }

  async function startMove() {
    if (!canMove()) return;
    const node = selected();
    setBusy(true);
    setStartError(null);
    setTarget(node);
    try {
      const started = await moves.start(props.sandboxId, node);
      setSteps(applyMove(emptySteps(), started));
      if (started.state === 'failed') {
        setFailure(failureNote({ message: started.error || 'The move failed.' }, props.currentNode));
        setPhase('failed');
      } else if (started.state === 'done') {
        setPhase('done');
      } else {
        setPhase('running');
        watch(started.id);
      }
      void refreshMove(started.id);
    } catch (failed) {
      setStartError(failed.message);
    } finally {
      setBusy(false);
    }
  }

  function backToChoose() {
    stopWatching();
    setPhase('choose');
    setFailure(null);
    setStartError(null);
    setSteps(emptySteps());
  }

  function closeDone() {
    props.onMoved?.();
    props.onClose?.();
  }

  const actions = () => {
    if (phase() === 'done') return [{ label: 'Close', tone: 'primary', onClick: closeDone }];
    if (phase() === 'failed') {
      return [
        { label: 'Close', tone: 'secondary', onClick: props.onClose },
        { label: 'Try another node', tone: 'primary', onClick: backToChoose },
      ];
    }
    return [{ label: 'Close', tone: 'secondary', onClick: props.onClose }];
  };

  return (
    <ConfirmDialog
      open={props.open}
      title={`Move ${props.sandboxName || 'this sandbox'}`}
      actions={actions()}
      onClose={props.onClose}
    >
      <Show when={phase() === 'choose'}>
        <p class="muted small">
          The sandbox keeps its identity, its policies and its folders. It stops while its data moves,
          and it comes back on the target started if it was running.
        </p>

        <Show when={nodesError()}>
          <p class="error">The nodes could not be read: {nodesError()}</p>
        </Show>
        <Show when={!nodesError() && !nodes().length}>
          <p class="muted small">There is no other node in this data center to move to.</p>
        </Show>

        <Show when={nodes().length > 0}>
          <p class="small">Target node</p>
          <div class="controls">
            <For each={nodes()}>
              {(node) => (
                <button
                  type="button"
                  class={selected() === node.name ? 'primary' : 'secondary'}
                  onClick={() => select(node)}
                >
                  {node.display_name || node.name}
                  <span class="muted small">
                    {' · '}
                    {node.status}
                    {node.container_gpu ? ' · GPU' : ' · no GPU'}
                  </span>
                </button>
              )}
            </For>
          </div>
        </Show>

        <Show when={checking() && selected()}>
          <p class="muted small">Checking {selected()}…</p>
        </Show>
        <Show when={preflightError()}>
          <p class="error">The preflight failed: {preflightError()}</p>
        </Show>

        <Show when={preflight()}>
          {(answer) => (
            <div>
              <Show when={answer().blockers?.length > 0}>
                <p class="error">This move cannot start:</p>
                <ul class="error small">
                  <For each={answer().blockers}>{(blocker) => <li>{blocker.message}</li>}</For>
                </ul>
              </Show>
              <Show when={answer().warnings?.length > 0}>
                <div class="warn-note small">
                  <For each={answer().warnings}>{(warning) => <div>{warning}</div>}</For>
                </div>
              </Show>
              <Show when={answer().carries?.length > 0}>
                <p class="small">Carried over</p>
                <ul class="small">
                  <For each={answer().carries}>{(carry) => <li class="mono small">{carry}</li>}</For>
                </ul>
              </Show>
              <Show when={answer().target}>
                <dl class="facts">
                  <dt>Target</dt>
                  <dd>{answer().target.name}</dd>
                  <dt>Reachable</dt>
                  <dd>
                    <span class={`badge ${answer().target.online ? 'ok' : 'bad'}`}>
                      <i />
                      {answer().target.online ? 'online' : 'offline'}
                    </span>
                  </dd>
                  <dt>GPU</dt>
                  <dd>{answer().target.container_gpu ? `yes · ${answer().target.gpu_count || 0}` : 'none'}</dd>
                  <dt>Memory free</dt>
                  <dd class="mono">{formatBytes(answer().target.free_memory_bytes)}</dd>
                </dl>
              </Show>
              <button type="button" class="primary" disabled={!canMove()} onClick={startMove}>
                {busy() ? 'Starting…' : 'Move now'}
              </button>
            </div>
          )}
        </Show>

        <Show when={startError()}>
          <p class="error">{startError()}</p>
        </Show>
      </Show>

      <Show when={phase() !== 'choose'}>
        <p class="small">
          Moving {props.sandboxName || 'the sandbox'} to {target()}. {progress(steps()).done} of{' '}
          {progress(steps()).total} steps done.
        </p>

        <Show when={phase() === 'running'}>
          <p class="muted small">The move continues if you close this window.</p>
        </Show>
        <Show when={phase() === 'done'}>
          <p class="notice">Moved to {target()}.</p>
        </Show>
        <Show when={phase() === 'failed' && failure()}>
          <p class="error">{failure()}</p>
        </Show>

        <ol>
          <For each={STEPS}>
            {(step) => (
              <li class="check" classList={{ fail: steps()[step] === 'failed' }}>
                <span>{mark(steps()[step])}</span>
                <span class={steps()[step] === 'pending' ? 'muted small' : undefined}>{STEP_LABELS[step]}</span>
              </li>
            )}
          </For>
        </ol>
      </Show>
    </ConfirmDialog>
  );
}
