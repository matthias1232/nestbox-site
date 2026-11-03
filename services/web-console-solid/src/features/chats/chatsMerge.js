// The live chat stream folded into a turn list.
//
// Two sources meet here: the rows the API stored, which are complete and
// authoritative, and the events the stream delivers while a call is still
// running, which are partial but arrive first. A running turn is keyed by its
// `request_ref`, because that is the only identity the stream has until the
// turn is stored.
//
// Every function is pure and returns a new state, so a Solid signal can hold it
// and a test can hold two states at once.

/** The stream's event names. The one place that spells them. */
export const EVENT = {
  started: 'chat_started',
  delta: 'chat_delta',
  done: 'chat_done',
};

/** A fresh merge state. */
export function emptyState() {
  return { byRef: new Map(), turns: [] };
}

/**
 * How many live turns the state keeps.
 *
 * The stream is unbounded by design; a console left open overnight must not
 * grow with it. The oldest running turns fall off first, and the stored list
 * still has them the moment the page is refreshed.
 */
export const MAX_LIVE_TURNS = 200;

function bounded(turns) {
  return turns.length > MAX_LIVE_TURNS ? turns.slice(0, MAX_LIVE_TURNS) : turns;
}

function asCount(value) {
  const number = Number(value);
  return Number.isFinite(number) ? number : 0;
}

/** The turn a `chat_started` event describes, before any answer exists. */
function startedTurn(event) {
  const model = event.model || null;
  return {
    id: `live:${event.request_ref}`,
    request_ref: event.request_ref,
    ts: event.ts || new Date().toISOString(),
    sandbox_id: event.sandbox_id || null,
    pending: true,
    delivered: event.delivered !== false,
    dlp: event.dlp || null,
    provider: event.provider || null,
    host: event.host || null,
    requested_model: model,
    served_model: model,
    user_text: event.user_text || '',
    assistant_text: '',
    tool_calls: [],
    input_tokens: asCount(event.input_tokens),
    output_tokens: asCount(event.output_tokens),
    cache_read: 0,
    cache_write: 0,
    route: [],
    incidents: [],
  };
}

function withoutRef(state, ref) {
  return state.turns.filter((turn) => turn.request_ref !== ref);
}

function replaceInList(turns, next) {
  let found = false;
  const list = turns.map((turn) => {
    const sameId = next.id && turn.id === next.id;
    const sameRef = next.request_ref && turn.request_ref === next.request_ref;
    if (!sameId && !sameRef) return turn;
    found = true;
    return next;
  });
  return found ? list : [next, ...list];
}

function applyStarted(state, event) {
  if (!event?.request_ref) return state;
  // A start for a reference already in the list is a reconnect replay: keep one
  // entry, take the newer metadata, and keep the text that already arrived
  // rather than blanking an answer the reader is watching.
  const known = state.byRef.get(event.request_ref);
  const base = startedTurn(event);
  const turn = known
    ? {
        ...base,
        assistant_text: known.assistant_text || '',
        input_tokens: Math.max(asCount(known.input_tokens), base.input_tokens),
        output_tokens: Math.max(asCount(known.output_tokens), base.output_tokens),
      }
    : base;
  return {
    byRef: new Map(state.byRef).set(event.request_ref, turn),
    turns: bounded(replaceInList(withoutRef(state, event.request_ref), turn)),
  };
}

function applyDelta(state, event) {
  if (!event?.request_ref) return state;
  const known = state.byRef.get(event.request_ref);
  // A delta without a start is not a turn: guessing one would put an answer
  // under a question nobody saw.
  if (!known) return state;
  const turn = {
    ...known,
    pending: true,
    assistant_text: event.text_so_far ?? known.assistant_text,
    input_tokens: event.input_tokens === undefined ? known.input_tokens : asCount(event.input_tokens),
    output_tokens: event.output_tokens === undefined ? known.output_tokens : asCount(event.output_tokens),
  };
  return {
    byRef: new Map(state.byRef).set(event.request_ref, turn),
    turns: replaceInList(state.turns, turn),
  };
}

function applyDone(state, event) {
  if (!event?.request_ref) return state;
  const known = state.byRef.get(event.request_ref);
  // The stored row is the authority; a done event for an unknown reference has
  // nothing to close.
  if (!known) return state;
  const turn = { ...known, pending: false, turn_id: event.turn_id || known.turn_id || null };
  return {
    byRef: new Map(state.byRef).set(event.request_ref, turn),
    turns: replaceInList(state.turns, turn),
  };
}

/**
 * Fold one stream event into the state.
 *
 * `event.type` is the discriminator; the caller adds it because the stream
 * hands each listener its payload without the event name. Unknown events are
 * ignored on purpose: a newer server may send more, and a console that breaks
 * on an extra event is a console that breaks during a release.
 */
export function applyEvent(state, event) {
  const current = state || emptyState();
  const type = event?.type;
  if (type === EVENT.started) return applyStarted(current, event);
  if (type === EVENT.delta) return applyDelta(current, event);
  if (type === EVENT.done) return applyDone(current, event);
  return current;
}

/**
 * Put the stored turn in place of its running version.
 *
 * The caller loads `chats.turn(turn_id)` when the turn ends; the stored row
 * carries the final text, the tokens and the findings, which the stream never
 * had in full.
 */
export function replaceTurn(state, turn) {
  const current = state || emptyState();
  if (!turn) return current;
  const next = { ...turn, pending: false };
  const byRef = new Map(current.byRef);
  if (next.request_ref) byRef.delete(next.request_ref);
  return { byRef, turns: bounded(replaceInList(current.turns, next)) };
}

/** The identity a list row uses: the stored id, else the live reference. */
export function turnKey(turn) {
  return turn?.id || turn?.request_ref || '';
}

/**
 * The stored page with the live turns on top.
 *
 * A live turn whose reference the page already contains is dropped: the stored
 * row is the same call, but complete.
 */
export function mergeLive(stored = [], live = []) {
  const known = new Set(stored.map((turn) => turn.request_ref).filter(Boolean));
  const fresh = live.filter((turn) => !turn.request_ref || !known.has(turn.request_ref));
  return [...fresh, ...stored];
}

/**
 * One list order for the reader.
 *
 * `asc` is the messenger order (oldest first, the running turn at the bottom).
 * The comparison falls back to the given order for equal timestamps, so two
 * turns of the same second do not swap on every render.
 */
export function sortTurns(turns = [], direction = 'asc') {
  const sign = direction === 'desc' ? -1 : 1;
  return turns
    .map((turn, index) => ({ turn, index, at: Date.parse(turn?.ts) || 0 }))
    .sort((left, right) => (left.at - right.at) * sign || left.index - right.index)
    .map((entry) => entry.turn);
}
