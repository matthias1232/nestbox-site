// The nine steps of a move and how a live event or a polled move changes them.
//
// Pure, so the progress view says the same thing whether a step arrived on the
// stream or from the five-second poll, and so the order of the steps is tested
// instead of being remembered in the JSX.

/** The steps in the order the mover runs them (ADR 0049). */
export const STEPS = [
  'check',
  'image',
  'snapshot',
  'stop',
  'delta',
  'restore',
  'cutover',
  'recreate',
  'verify',
];

/** What each step does, in words a person who never read the mover can follow. */
export const STEP_LABELS = {
  check: 'Check the target',
  image: 'Have the image there',
  snapshot: 'Snapshot the folders while it runs',
  stop: 'Stop the sandbox',
  delta: 'Send what changed since',
  restore: 'Restore the folders on the target',
  cutover: 'Move the folders to the target',
  recreate: 'Create the sandbox on the target',
  verify: 'Verify it runs there',
};

const STEP_STATES = ['pending', 'running', 'done', 'failed'];

/** A fresh state map: every step pending. */
export function emptySteps() {
  const steps = {};
  for (const step of STEPS) steps[step] = 'pending';
  return steps;
}

/** Whether a name is one of the nine steps. */
export function isStep(step) {
  return STEPS.includes(step);
}

/**
 * Apply one live move event (`detail` is `{step, state, message}`).
 *
 * An event that names no known step or no known state is ignored rather than
 * guessed at, so a message from a newer mover cannot paint a step green. A step
 * that is already done is never set back to pending: events and polls arrive in
 * either order, and a step the mover finished stays finished.
 */
export function applyEvent(steps, detail) {
  const next = { ...steps };
  const step = detail?.step;
  if (!isStep(step)) return next;
  const state = String(detail.state || '');
  if (!STEP_STATES.includes(state)) return next;
  if (next[step] === 'done' && (state === 'pending' || state === 'running')) return next;
  next[step] = state;
  return next;
}

/**
 * Derive the state map from a polled move (`state`, `step` of the move row).
 *
 * The row knows the current step but not the history, so every earlier step
 * counts as done, the current one takes the move's outcome, and a stale
 * "running" on a later step goes back to pending.
 */
export function applyMove(steps, move) {
  const next = { ...steps };
  const current = STEPS.indexOf(move?.step);
  if (current < 0) return next;
  const failed = move.state === 'failed';
  const done = move.state === 'done';
  STEPS.forEach((step, at) => {
    if (at < current) next[step] = 'done';
    else if (at === current) next[step] = failed ? 'failed' : done ? 'done' : 'running';
    else if (next[step] === 'running' || next[step] === 'failed') next[step] = 'pending';
  });
  return next;
}

/** How far along a state map is, for the line above the steps. */
export function progress(steps) {
  let done = 0;
  let failed = 0;
  let running = 0;
  let current = null;
  for (const step of STEPS) {
    const state = steps?.[step];
    if (state === 'done') done += 1;
    else if (state === 'failed') failed += 1;
    else if (state === 'running') running += 1;
    if (current === null && state !== 'done') current = step;
  }
  return { done, failed, running, total: STEPS.length, current };
}

/**
 * Whether a move row says the sandbox was put back on the source node.
 *
 * The mover writes its own sentence into `error`, and the step log keeps what
 * happened, so both are read: saying "rolled back" when it did not happen would
 * send the user looking at the wrong machine.
 */
export function rolledBack(move) {
  const text = `${move?.error || ''} ${JSON.stringify(move?.log || [])}`.toLowerCase();
  return text.includes('rolled back') || text.includes('rollback');
}

/** The sentence after a failed move, with the rollback sentence only when it happened. */
export function failureNote(detail, sourceNode) {
  const message = String(detail?.message || 'The move failed.');
  const rolled = detail?.rolled_back === true || detail?.rollback === true;
  if (!rolled) return message;
  return sourceNode ? `${message} Rolled back to ${sourceNode}.` : `${message} Rolled back to the source node.`;
}

/**
 * The nodes a move may go to: every node but the current one, online first.
 *
 * The current node is not a move, and an offline node would only produce a
 * blocker the user has to read twice, so it comes last but stays visible.
 */
export function orderNodes(nodes, currentNode) {
  const others = (nodes || []).filter((node) => node && node.name && node.name !== currentNode);
  const rank = (node) => (node.status === 'online' ? 0 : node.status === 'degraded' ? 1 : 2);
  return others.sort((left, right) => {
    const byRank = rank(left) - rank(right);
    if (byRank !== 0) return byRank;
    return String(left.display_name || left.name).localeCompare(String(right.display_name || right.name));
  });
}

/** A byte count as someone would say it; "unknown" when the node was never measured. */
export function formatBytes(bytes) {
  if (bytes === null || bytes === undefined || bytes === '') return 'unknown';
  const value = Number(bytes);
  if (!Number.isFinite(value)) return 'unknown';
  const GIB = 1024 ** 3;
  const MIB = 1024 ** 2;
  if (value >= GIB) return `${(value / GIB).toFixed(1)} GiB`;
  if (value >= MIB) return `${Math.round(value / MIB)} MiB`;
  return `${Math.max(0, Math.round(value))} B`;
}
