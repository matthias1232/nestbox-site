// The display layout of one desktop sandbox, edited in the console and applied
// live (U10b, U10c).
//
// One owner, one truth: this hook holds the layout the sandbox reported, lets the
// arrangement and the monitor tabs change it locally at once, and writes it back
// after **400 ms** of quiet — so dragging a monitor across the SVG is one request,
// not sixty. Only layouts the server would accept are sent; an invalid one stays
// local and the status line names the first rule it breaks.
//
// A layout can also change under us: an agent through the desktop-control MCP
// server, or another operator. Two things keep that honest:
//
//   * every write carries `expected_applied_at`, so a stale write is refused with
//     `409 display.changed` and this hook adopts the layout that is really there
//     instead of overwriting somebody else's work;
//   * the layout is re-read on the live `control` event with `what: "display"` and
//     every 10 s while the desktop view is open, so the tabs and the arrangement
//     follow by themselves.
import { createSignal, onCleanup, onMount } from 'solid-js';
import * as displayApi from '../../api/display.js';
import { useLive } from '../../state/live.jsx';
import {
  addMonitor,
  describeApplied,
  moveMonitor,
  removeMonitor,
  renameMonitor,
  resizeMonitor,
  rotateMonitor,
  setMode,
  setPrimary,
  validate,
} from './displayLayout.js';

/** How long the arrangement has to be quiet before the layout is sent. */
export const QUIET_MS = 400;

/** How often an open desktop re-reads the layout, so an agent's change appears. */
export const REFRESH_MS = 10000;

/**
 * Keep the display layout of `sandboxId` (a string or a getter) applied live.
 *
 * Returns the layout and its status, plus one method per edit. Every edit is
 * local at once and sent after the quiet period; `status()` says what happened.
 */
export function useDisplay(sandboxId) {
  const live = useLive();
  const id = () => (typeof sandboxId === 'function' ? sandboxId() : sandboxId);

  const [layout, setLayout] = createSignal(null);
  const [framebuffer, setFramebuffer] = createSignal(null);
  const [appliedAt, setAppliedAt] = createSignal(null);
  const [appliedBy, setAppliedBy] = createSignal(null);
  const [problems, setProblems] = createSignal([]);
  const [status, setStatus] = createSignal({ kind: 'loading', text: 'Reading the display …' });
  const [revision, setRevision] = createSignal(0);

  let timer = null;
  let applying = false;
  let alive = true;

  function cancel() {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  }

  // Take the sandbox's answer as the truth. `revision` wakes the cropped view, so
  // a tab re-crops after an apply even when the geometry stayed the same.
  function adopt(answer) {
    const next = answer?.layout || answer;
    setLayout(next);
    setFramebuffer(answer?.framebuffer || null);
    setAppliedAt(answer?.applied_at || null);
    setAppliedBy(answer?.applied_by || null);
    setProblems([]);
    setRevision((count) => count + 1);
    return next;
  }

  async function applyNow() {
    const target = id();
    const wanted = layout();
    timer = null;
    if (!alive || !target || !wanted) return;
    // A write is already on its way: this edit waits its turn rather than being
    // dropped, so the last thing the operator did is what the sandbox ends with.
    if (applying) {
      timer = setTimeout(applyNow, QUIET_MS);
      return;
    }
    const started = Date.now();
    applying = true;
    setStatus({ kind: 'applying', text: 'Applying…' });
    try {
      const answer = await displayApi.set(target, wanted, appliedAt());
      if (!alive || target !== id()) return;
      const applied = adopt(answer) || wanted;
      setStatus({ kind: 'applied', text: describeApplied(applied, Date.now() - started) });
    } catch (error) {
      if (!alive || target !== id()) return;
      if (error?.code === 'display.changed' && error.details?.layout) {
        // Somebody else changed the screen between our read and our write. Their
        // layout is the truth now; ours would undo work that is already visible.
        adopt(error.details);
        setStatus({
          kind: 'changed',
          text: `Changed by ${error.details.applied_by || 'someone'} just now`,
        });
      } else {
        setStatus({ kind: 'error', text: error?.message || 'The display layout could not be applied.' });
      }
    } finally {
      applying = false;
    }
  }

  function schedule() {
    cancel();
    setStatus({ kind: 'applying', text: 'Applying…' });
    timer = setTimeout(applyNow, QUIET_MS);
  }

  /** Change the layout locally and write it back once the edits stop. */
  function edit(next) {
    if (!next) return;
    setLayout(next);
    const found = validate(next);
    setProblems(found);
    if (found.length > 0) {
      cancel();
      setStatus({ kind: 'invalid', text: found[0] });
      return;
    }
    schedule();
  }

  /** Re-read the layout; called on mount, every 10 s and on the live display event. */
  async function refresh() {
    const target = id();
    // A local edit is on its way: the server would answer with the old layout and
    // undo what the operator is doing right now.
    if (!alive || !target || applying || timer !== null) return;
    try {
      const answer = await displayApi.get(target);
      if (!alive || target !== id() || applying || timer !== null) return;
      const previous = JSON.stringify(layout());
      const changed = (answer?.applied_at || null) !== appliedAt();
      adopt(answer);
      if (changed && previous && JSON.stringify(answer.layout) !== previous) {
        setStatus({ kind: 'changed', text: `Changed by ${answer.applied_by || 'someone'} just now` });
      } else if (status().kind === 'loading') {
        setStatus({ kind: 'idle', text: '' });
      }
    } catch (error) {
      // A stopped sandbox has no display to read; the picture says so already.
      if (!layout()) setStatus({ kind: 'error', text: error?.message || 'The display could not be read.' });
    }
  }

  onMount(() => {
    refresh();
    const timerId = setInterval(() => {
      if (!document.hidden) refresh();
    }, REFRESH_MS);
    // An agent's change arrives as one event; the layout is re-read rather than
    // taken from the event, because the event is a notification, not the truth.
    const stop = live.on('control', (payload) => {
      if (payload?.what !== 'display') return;
      if (payload.sandbox_id && payload.sandbox_id !== id()) return;
      refresh();
    });
    onCleanup(() => {
      alive = false;
      cancel();
      clearInterval(timerId);
      stop();
    });
  });

  return {
    layout,
    framebuffer,
    appliedAt,
    appliedBy,
    problems,
    status,
    revision,
    refresh,
    edit,
    add: (spec) => edit(addMonitor(layout(), spec)),
    remove: (monitorId) => edit(removeMonitor(layout(), monitorId)),
    resize: (monitorId, width, height) => edit(resizeMonitor(layout(), monitorId, width, height)),
    rotate: (monitorId) => edit(rotateMonitor(layout(), monitorId)),
    rename: (monitorId, name) => edit(renameMonitor(layout(), monitorId, name)),
    primary: (monitorId) => edit(setPrimary(layout(), monitorId)),
    mode: (mode) => edit(setMode(layout(), mode)),
    // One drag: the caller's snap already happened in the arrangement, so this is
    // the same edit as any other and shares the 400 ms quiet period.
    move: (monitorId, x, y) => edit(moveMonitor(layout(), monitorId, x, y)),
  };
}
