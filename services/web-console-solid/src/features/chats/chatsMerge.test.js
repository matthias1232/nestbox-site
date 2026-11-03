import test from 'node:test';
import assert from 'node:assert/strict';
import {
  EVENT,
  applyEvent,
  emptyState,
  mergeLive,
  replaceTurn,
  sortTurns,
  turnKey,
} from './chatsMerge.js';

const started = (over = {}) => ({
  type: EVENT.started,
  request_ref: 'r1',
  ts: '2026-10-01T10:00:00Z',
  sandbox_id: 's1',
  model: 'claude-sonnet',
  provider: 'anthropic',
  dlp: 'replaced',
  delivered: true,
  user_text: 'bitte <IBAN_ab12cd34> prüfen',
  ...over,
});

test('chat_started opens a running turn', () => {
  const state = applyEvent(emptyState(), started());
  assert.equal(state.turns.length, 1);
  const turn = state.turns[0];
  assert.equal(turn.pending, true);
  assert.equal(turn.request_ref, 'r1');
  assert.equal(turn.user_text, 'bitte <IBAN_ab12cd34> prüfen');
  assert.equal(turn.served_model, 'claude-sonnet');
  assert.equal(state.byRef.get('r1'), turn);
});

test('start, delta and done in order build one turn', () => {
  let state = emptyState();
  state = applyEvent(state, started());
  state = applyEvent(state, {
    type: EVENT.delta,
    request_ref: 'r1',
    text_so_far: 'Ich sehe ',
    input_tokens: 12,
    output_tokens: 3,
  });
  state = applyEvent(state, {
    type: EVENT.delta,
    request_ref: 'r1',
    text_so_far: 'Ich sehe eine IBAN.',
    input_tokens: 12,
    output_tokens: 9,
  });

  assert.equal(state.turns.length, 1);
  assert.equal(state.turns[0].assistant_text, 'Ich sehe eine IBAN.');
  assert.equal(state.turns[0].input_tokens, 12);
  assert.equal(state.turns[0].output_tokens, 9);
  assert.equal(state.turns[0].pending, true);

  state = applyEvent(state, { type: EVENT.done, request_ref: 'r1', turn_id: 't1' });
  assert.equal(state.turns.length, 1);
  assert.equal(state.turns[0].pending, false);
  assert.equal(state.turns[0].turn_id, 't1');
  assert.equal(state.turns[0].assistant_text, 'Ich sehe eine IBAN.');
});

test('a delta without a start is ignored, not guessed', () => {
  const before = emptyState();
  const after = applyEvent(before, {
    type: EVENT.delta,
    request_ref: 'ghost',
    text_so_far: 'woher?',
  });
  assert.equal(after, before);
  assert.equal(after.turns.length, 0);
});

test('a done without a start is ignored', () => {
  const before = emptyState();
  const after = applyEvent(before, { type: EVENT.done, request_ref: 'ghost', turn_id: 't9' });
  assert.equal(after, before);
});

test('an unknown event leaves the state alone', () => {
  const state = applyEvent(emptyState(), started());
  assert.equal(applyEvent(state, { type: 'chat_paused', request_ref: 'r1' }), state);
  assert.equal(applyEvent(state, null), state);
});

test('a replayed start does not double the turn or blank its text', () => {
  let state = applyEvent(emptyState(), started());
  state = applyEvent(state, { type: EVENT.delta, request_ref: 'r1', text_so_far: 'halb' });
  state = applyEvent(state, started());
  assert.equal(state.turns.length, 1);
  assert.equal(state.byRef.size, 1);
  assert.equal(state.turns[0].assistant_text, 'halb');
});

test('the stored turn replaces the running one', () => {
  let state = applyEvent(emptyState(), started());
  state = applyEvent(state, { type: EVENT.done, request_ref: 'r1', turn_id: 't1' });
  state = replaceTurn(state, {
    id: 't1',
    request_ref: 'r1',
    ts: '2026-10-01T10:00:00Z',
    user_text: 'bitte <IBAN_ab12cd34> prüfen',
    assistant_text: 'Die IBAN ist ersetzt worden.',
    delivered: true,
    tool_calls: [],
    incidents: [{ detector: 'iban', action: 'replaced' }],
  });

  assert.equal(state.turns.length, 1);
  assert.equal(state.turns[0].id, 't1');
  assert.equal(state.turns[0].pending, false);
  assert.equal(state.turns[0].assistant_text, 'Die IBAN ist ersetzt worden.');
  assert.equal(state.byRef.has('r1'), false);
});

test('a stored turn without a running twin is added once', () => {
  let state = applyEvent(emptyState(), started());
  const stored = { id: 't2', request_ref: 'other', assistant_text: 'x' };
  state = replaceTurn(state, stored);
  state = replaceTurn(state, stored);
  assert.equal(state.turns.length, 2);
  assert.equal(state.turns.find((turn) => turn.id === 't2').pending, false);
});

test('the live page drops a turn the stored page already has', () => {
  const storedPage = [{ id: 't1', request_ref: 'r1', assistant_text: 'fertig' }];
  const liveTurn = { id: 'live:r1', request_ref: 'r1', assistant_text: 'teil' };
  const liveOnly = { id: 'live:r2', request_ref: 'r2', assistant_text: '' };
  const merged = mergeLive(storedPage, [liveTurn, liveOnly]);
  assert.equal(merged.length, 2);
  assert.deepEqual(merged.map((turn) => turn.id), ['live:r2', 't1']);
});

test('the messenger order is oldest first, the running turn last', () => {
  const turns = [
    { id: 'b', ts: '2026-10-01T10:05:00Z', pending: true },
    { id: 'a', ts: '2026-10-01T10:00:00Z' },
    { id: 'c', ts: '2026-10-01T10:03:00Z' },
  ];
  assert.deepEqual(sortTurns(turns, 'asc').map((turn) => turn.id), ['a', 'c', 'b']);
  assert.deepEqual(sortTurns(turns, 'desc').map((turn) => turn.id), ['b', 'c', 'a']);
  // The input list is untouched: a sort is a view, not a mutation.
  assert.equal(turns[0].id, 'b');
});

test('turns of the same second keep the order they arrived in', () => {
  const turns = [
    { id: 'first', ts: '2026-10-01T10:00:00Z' },
    { id: 'second', ts: '2026-10-01T10:00:00Z' },
  ];
  assert.deepEqual(sortTurns(turns, 'asc').map((turn) => turn.id), ['first', 'second']);
});

test('a turn is keyed by its stored id, else by its live reference', () => {
  assert.equal(turnKey({ id: 't1', request_ref: 'r1' }), 't1');
  assert.equal(turnKey({ id: 'live:r1', request_ref: 'r1' }), 'live:r1');
  assert.equal(turnKey({ request_ref: 'r1' }), 'r1');
  assert.equal(turnKey(null), '');
});
