import test from 'node:test';
import assert from 'node:assert/strict';
import { actionsFor, paramsValid, paramsFor, confirmText, needsDialog } from './bulkActions.js';

test('a member sees only start, stop, restart and allow all', () => {
  assert.deepEqual(actionsFor('sandbox', 'member').map((a) => a.id), ['start', 'stop', 'restart', 'allow_all']);
});

test('an admin sees every sandbox action, delete is dangerous', () => {
  const all = actionsFor('sandbox', 'admin');
  assert.equal(all.length, 9);
  assert.equal(all.find((a) => a.id === 'delete').danger, true);
});

test('viewers and unknown roles get nothing', () => {
  assert.deepEqual(actionsFor('sandbox', 'viewer'), []);
  assert.deepEqual(actionsFor('sandbox', 'x'), []);
  assert.deepEqual(actionsFor('other', 'admin'), []);
});

test('nodes offer deploy and probe to operators only', () => {
  assert.deepEqual(actionsFor('node', 'operator').map((a) => a.id), ['deploy_model', 'probe_all']);
  assert.deepEqual(actionsFor('node', 'member'), []);
});

test('paramsValid checks the input each action needs', () => {
  assert.equal(paramsValid('stop', {}), true);
  assert.equal(paramsValid('egress_mode', { mode: 'hybrid' }), true);
  assert.equal(paramsValid('egress_mode', { mode: 'nope' }), false);
  assert.equal(paramsValid('assign_policy', {}), false);
  assert.equal(paramsValid('move', { node: 'gpu-08' }), true);
  assert.equal(paramsValid('delete', { confirm: 'Delete' }), false);
  assert.equal(paramsValid('delete', { confirm: 'delete' }), true);
  assert.equal(paramsValid('deploy_model', { model_key: ' ' }), false);
});

test('paramsFor fixes the allow-all duration', () => {
  assert.deepEqual(paramsFor('allow_all', { x: 1 }), { minutes: 15 });
  assert.deepEqual(paramsFor('move', { node: 'a' }), { node: 'a' });
});

test('confirmText counts and pluralises', () => {
  assert.equal(confirmText('stop', 12), 'Stop 12 sandboxes?');
  assert.equal(confirmText('delete', 1), 'Delete 1 sandbox?');
  assert.equal(confirmText('probe_all', 3, 'node'), 'Probe 3 nodes?');
});

test('start needs no dialog, delete does', () => {
  assert.equal(needsDialog({ id: 'start' }), false);
  assert.equal(needsDialog({ id: 'delete', danger: true, needs: 'confirm' }), true);
});
