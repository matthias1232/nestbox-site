import test from 'node:test';
import assert from 'node:assert/strict';
import { health, strategyLabel, canRemove, modelsWithInstalls, pickableInstalls, usersFromError } from './clusterHealth.js';

const member = (id, ok = true) => ({ install_id: id, state: ok ? 'installed' : 'failed', provider_status: ok ? 'reachable' : 'unreachable' });

test('all members healthy is ok and says so', () => {
  const h = health({ members: [member('a'), member('b')], healthy_members: 2 });
  assert.deepEqual(h, { tone: 'ok', text: '2 of 2 members healthy' });
});

test('some healthy is a warning with the count', () => {
  const h = health({ members: [member('a'), member('b'), member('c'), member('d', false)] });
  assert.equal(h.tone, 'warn');
  assert.equal(h.text, '3 of 4 members healthy');
});

test('none healthy is bad and an empty cluster says no members', () => {
  assert.equal(health({ members: [member('a', false)], healthy_members: 0 }).tone, 'bad');
  assert.equal(health({ members: [] }).text, 'No members');
});

test('strategy labels are readable and unknown ids pass through', () => {
  assert.equal(strategyLabel('least_loaded'), 'Least loaded');
  assert.equal(strategyLabel('x'), 'x');
});

test('the last member cannot be removed', () => {
  assert.equal(canRemove({ members: [member('a')] }, 'a'), false);
  assert.equal(canRemove({ members: [member('a'), member('b')] }, 'a'), true);
  assert.equal(canRemove({ members: [member('a'), member('b')] }, 'zzz'), false);
});

test('models list only keys that have installs; pickable skips members and broken installs', () => {
  const rows = [
    { id: '1', model_key: 'm1', model_name: 'Beta', state: 'installed' },
    { id: '2', model_key: 'm1', model_name: 'Beta', state: 'installed' },
    { id: '3', model_key: 'm1', model_name: 'Beta', state: 'failed' },
    { id: '4', model_key: 'm2', model_name: 'Alpha', state: 'installed' },
  ];
  assert.deepEqual(modelsWithInstalls(rows).map((m) => m.key), ['m2', 'm1']);
  assert.deepEqual(pickableInstalls(rows, 'm1', ['1']).map((r) => r.id), ['2']);
});

test('sandboxes using a cluster are read from the 409 details', () => {
  assert.deepEqual(usersFromError({ details: { sandboxes: ['a', { name: 'b' }] } }), ['a', 'b']);
  assert.deepEqual(usersFromError(new Error('x')), []);
});
