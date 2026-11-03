import test from 'node:test';
import assert from 'node:assert/strict';
import { rolesFor, viaLabel, canRemove, roleSentence, canChangeRole, addableUsers, inviteLink } from './permissions.js';

test('a sandbox scope only offers member', () => {
  assert.deepEqual(rolesFor('sandbox'), ['member']);
  assert.equal(rolesFor('environment').length, 4);
  assert.deepEqual(rolesFor('organization'), []);
});

test('via labels', () => {
  assert.equal(viaLabel('grant'), 'Sandbox grant');
  assert.equal(viaLabel('organization'), 'From organization');
});

test('the last admin and yourself cannot be removed', () => {
  const rows = [
    { user_id: 'a', role: 'admin', via: 'direct' },
    { user_id: 'b', role: 'member', via: 'direct' },
  ];
  assert.equal(canRemove(rows[0], 'x', rows), false);
  assert.equal(canRemove(rows[1], 'b', rows), false);
  assert.equal(canRemove(rows[1], 'a', rows), true);
  rows.push({ user_id: 'c', role: 'admin', via: 'direct' });
  assert.equal(canRemove(rows[0], 'x', rows), true);
});

test('inherited rows cannot be removed or edited here', () => {
  const row = { user_id: 'a', role: 'member', via: 'organization' };
  assert.equal(canRemove(row, 'x', [row]), false);
  assert.equal(canChangeRole(row, 'x', 'environment'), false);
});

test('role sentences and helpers', () => {
  assert.match(roleSentence('member'), /own sandboxes/);
  assert.deepEqual(addableUsers([{ id: 'a' }, { id: 'b' }], [{ user_id: 'a' }]), [{ id: 'b' }]);
  assert.equal(inviteLink('https://h', 't'), 'https://h/?invite=t');
});
