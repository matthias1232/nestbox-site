import test from 'node:test';
import assert from 'node:assert/strict';
import { PER_REFRESH, clip, flatten, groupTurns, hasMore, turnOf } from './livechats.js';

test('clip keeps a short text as one line', () => {
  assert.equal(clip('  hello\n  world ', 40), 'hello world');
  assert.equal(clip('', 10), '');
  assert.equal(clip(null, 10), '');
});

test('clip cuts a long text at the limit and marks it', () => {
  const text = 'a'.repeat(20);
  assert.equal(clip(text, 10), `${'a'.repeat(10)}…`);
  assert.equal(clip(text, 10).length, 11);
});

test('hasMore is true only when clip would cut', () => {
  assert.equal(hasMore('abc', 3), false);
  assert.equal(hasMore('abcd', 3), true);
  assert.equal(hasMore('a\n b', 3), false);
});

test('flatten collapses whitespace', () => {
  assert.equal(flatten('a\t\n b'), 'a b');
});

test('one refresh fetches a handful of bodies, not all of them', () => {
  assert.equal(PER_REFRESH, 6);
});

test('a turn carries the session, the question, the answer and the tokens', () => {
  const item = { id: 'e1', ts: '2026-10-01T10:00:00Z', host: 'api.anthropic.com', path: '/v1/messages', status: 200 };
  const body = {
    request_body: JSON.stringify({ model: 'claude-sonnet', messages: [{ role: 'user', content: 'mach das' }], metadata: { user_id: '{"session_id":"27a41161-b693-419a-850f-a5f2565bd848"}' } }),
    response_body: JSON.stringify({ content: [{ type: 'text', text: 'gemacht' }], usage: { input_tokens: 7, output_tokens: 3 } }),
    truncated: false,
  };
  const turn = turnOf(item, body);
  assert.equal(turn.session, '27a41161-b693-419a-850f-a5f2565bd848');
  assert.equal(turn.ask.prompt, 'mach das');
  assert.equal(turn.answer.text, 'gemacht');
  assert.deepEqual(turn.answer.tokens, { input: 7, output: 3 });
  assert.equal(turn.unreadable, undefined);
});

test('a body that cannot be read stays as an unreadable turn', () => {
  const turn = turnOf({ id: 'e2', ts: '2026-10-01T10:00:00Z' }, null);
  assert.equal(turn.unreadable, true);
  assert.equal(turn.session, null);
});

test('groupTurns puts one session together, newest session first', () => {
  const turns = [
    { id: 'a', ts: '2026-10-01T09:00:00Z', session: 's1', ask: { model: 'm1', prompt: 'first' }, answer: { text: 'one' } },
    { id: 'b', ts: '2026-10-01T11:00:00Z', session: 's2', ask: { model: 'm2', prompt: 'other' }, answer: { text: 'two' } },
    { id: 'c', ts: '2026-10-01T10:00:00Z', session: 's1', ask: { model: 'm1', prompt: 'second' }, answer: { text: 'three' } },
  ];
  const groups = groupTurns(turns);
  assert.equal(groups.length, 2);
  assert.equal(groups[0].session, 's2');
  assert.equal(groups[1].session, 's1');
  assert.deepEqual(groups[1].turns.map((turn) => turn.id), ['c', 'a']);
  assert.equal(groups[1].last_ts, '2026-10-01T10:00:00Z');
  assert.equal(groups[1].model, 'm1');
  assert.equal(groups[1].turns.length, 2);
});

test('a title answer names its group and is not a turn', () => {
  const turns = [
    { id: 't', ts: '2026-10-01T10:00:01Z', session: 's1', ask: { model: 'm', prompt: '' }, answer: { text: '{"title":"Login fix"}' }, title: 'Login fix' },
    { id: 'a', ts: '2026-10-01T10:00:00Z', session: 's1', ask: { model: 'm', prompt: 'fix it' }, answer: { text: 'done' } },
  ];
  const [group] = groupTurns(turns);
  assert.equal(group.title, 'Login fix');
  assert.deepEqual(group.turns.map((turn) => turn.id), ['a']);
});

test('a call with neither a question nor an answer is dropped', () => {
  const groups = groupTurns([
    { id: 'x', ts: '2026-10-01T10:00:00Z', session: 's1', ask: { model: 'm' }, answer: { text: '' } },
  ]);
  assert.deepEqual(groups, []);
});

test('calls without a session become one group of their own', () => {
  const groups = groupTurns([
    { id: 'a', ts: '2026-10-01T10:00:00Z', session: null, ask: { prompt: 'x' }, answer: { text: 'y' } },
  ]);
  assert.equal(groups.length, 1);
  assert.equal(groups[0].session, 'none');
});

test('groupTurns tolerates no turns at all', () => {
  assert.deepEqual(groupTurns(null), []);
  assert.deepEqual(groupTurns([]), []);
});
