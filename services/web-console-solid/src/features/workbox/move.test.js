import test from 'node:test';
import assert from 'node:assert/strict';

import {
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

test('a move has nine steps in the order the mover runs them', () => {
  assert.deepEqual(STEPS, ['check', 'image', 'snapshot', 'stop', 'delta', 'restore', 'cutover', 'recreate', 'verify']);
  assert.deepEqual(emptySteps(), Object.fromEntries(STEPS.map((step) => [step, 'pending'])));
});

test('an event paints only its own step', () => {
  const running = applyEvent(emptySteps(), { step: 'snapshot', state: 'running' });
  assert.equal(running.snapshot, 'running');
  assert.equal(running.check, 'pending');
  const done = applyEvent(running, { step: 'snapshot', state: 'done' });
  assert.equal(done.snapshot, 'done');
});

test('an event with an unknown step or state changes nothing', () => {
  const steps = applyEvent(emptySteps(), { step: 'check', state: 'done' });
  assert.equal(applyEvent(steps, { step: 'teleport', state: 'done' }).check, 'done');
  assert.equal(applyEvent(steps, { step: 'check', state: 'green' }).check, 'done');
  assert.equal(applyEvent(steps, {}).check, 'done');
  assert.equal(applyEvent(steps, null).check, 'done');
});

test('a late event does not undo a finished step', () => {
  const steps = applyEvent(emptySteps(), { step: 'stop', state: 'done' });
  assert.equal(applyEvent(steps, { step: 'stop', state: 'running' }).stop, 'done');
  // A failure may still be reported for a step the map believed done.
  assert.equal(applyEvent(steps, { step: 'stop', state: 'failed' }).stop, 'failed');
});

test('a polled move counts every earlier step as done', () => {
  const steps = applyMove(emptySteps(), { state: 'running', step: 'restore' });
  assert.deepEqual([steps.check, steps.snapshot, steps.stop, steps.delta], ['done', 'done', 'done', 'done']);
  assert.equal(steps.restore, 'running');
  assert.equal(steps.cutover, 'pending');
});

test('a polled move drops a stale running mark from a later step', () => {
  const ahead = applyEvent(emptySteps(), { step: 'verify', state: 'running' });
  const steps = applyMove(ahead, { state: 'running', step: 'stop' });
  assert.equal(steps.stop, 'running');
  assert.equal(steps.verify, 'pending');
});

test('a polled done move leaves the last step done and nothing running', () => {
  const steps = applyMove(emptySteps(), { state: 'done', step: 'verify' });
  assert.equal(steps.verify, 'done');
  assert.ok(STEPS.every((step) => steps[step] === 'done'));
});

test('a polled failed move marks the step it stopped at', () => {
  const steps = applyMove(emptySteps(), { state: 'failed', step: 'restore' });
  assert.equal(steps.restore, 'failed');
  assert.equal(steps.cutover, 'pending');
  assert.equal(progress(steps).failed, 1);
});

test('progress counts what is done and names the first step that is not', () => {
  let steps = emptySteps();
  steps = applyEvent(steps, { step: 'check', state: 'done' });
  steps = applyEvent(steps, { step: 'image', state: 'done' });
  steps = applyEvent(steps, { step: 'snapshot', state: 'running' });
  assert.deepEqual(progress(steps), { done: 2, failed: 0, running: 1, total: 9, current: 'snapshot' });
  assert.equal(progress(emptySteps()).current, 'check');
});

test('a rollback is only claimed when the mover said so', () => {
  assert.equal(failureNote({ message: 'The target ran out of disk.' }, 'node-a'), 'The target ran out of disk.');
  assert.equal(
    failureNote({ message: 'The restore failed.', rolled_back: true }, 'node-a'),
    'The restore failed. Rolled back to node-a.',
  );
  assert.equal(
    failureNote({ message: 'The restore failed.', rolled_back: true }, null),
    'The restore failed. Rolled back to the source node.',
  );
  assert.equal(failureNote({}, 'node-a'), 'The move failed.');
});

test('a move row is read for its rollback sentence', () => {
  assert.equal(rolledBack({ error: 'cutover failed; rolled back to node-a' }), true);
  assert.equal(rolledBack({ error: 'verify failed', log: [{ step: 'cutover', detail: 'rollback started' }] }), true);
  assert.equal(rolledBack({ error: 'verify failed', log: [] }), false);
  assert.equal(rolledBack(null), false);
});

test('the current node is not offered and online nodes come first', () => {
  const nodes = [
    { name: 'off-1', status: 'offline', display_name: 'offline box' },
    { name: 'here', status: 'online', display_name: 'this one' },
    { name: 'b-on', status: 'online', display_name: 'b box' },
    { name: 'a-on', status: 'online', display_name: 'a box' },
    { name: 'deg', status: 'degraded', display_name: 'degraded box' },
  ];
  assert.deepEqual(orderNodes(nodes, 'here').map((node) => node.name), ['a-on', 'b-on', 'deg', 'off-1']);
  assert.deepEqual(orderNodes(nodes, null).length, 5);
});

test('a free-memory number reads as GiB, MiB, or unknown', () => {
  assert.equal(formatBytes(16 * 1024 ** 3), '16.0 GiB');
  assert.equal(formatBytes(1536 * 1024 ** 2), '1.5 GiB');
  assert.equal(formatBytes(512 * 1024 ** 2), '512 MiB');
  assert.equal(formatBytes(2048), '2048 B');
  assert.equal(formatBytes(null), 'unknown');
  assert.equal(formatBytes(undefined), 'unknown');
});
