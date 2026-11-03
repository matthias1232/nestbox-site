import test from 'node:test';
import assert from 'node:assert/strict';
import { toRows, flip, setPath, add, remove, validatePath, errors, bodyFor, changed, policyLists, setPolicyLists, suggestPath } from './mountEditor.js';

const effective = { roots: [{ root_id: 'a', slug: 'portal', kind: 'project', owner: 'ada', node: 'n1', mount_path: '/work/portal', read_only: false, state: 'mounted' }] };

test('flip toggles read-only on one row', () => {
  const rows = flip(toRows(effective), 'a');
  assert.equal(rows[0].read_only, true);
  assert.equal(flip(rows, 'a')[0].read_only, false);
});

test('add suggests a path and does not add twice', () => {
  const rows = add(toRows(effective), { root_id: 'b', slug: 'data', kind: 'project' });
  assert.equal(rows[1].mount_path, '/work/data');
  assert.equal(add(rows, { root_id: 'b', slug: 'data' }).length, 2);
  assert.equal(suggestPath({ slug: 'x', kind: 'agent_home' }), '/home/nestbox/.agent-state/x');
});

test('remove drops the row', () => {
  assert.equal(remove(toRows(effective), 'a').length, 0);
});

test('validatePath rejects relative, dotdot and duplicates', () => {
  assert.ok(validatePath('work'));
  assert.ok(validatePath('/a/../b'));
  assert.ok(validatePath('/a', ['/a']));
  assert.equal(validatePath('/work/x'), null);
});

test('errors flags duplicate paths on both rows', () => {
  let rows = add(toRows(effective), { root_id: 'b', slug: 'portal' });
  assert.deepEqual(Object.keys(errors(rows)).sort(), ['a', 'b']);
  rows = setPath(rows, 'b', '/work/other');
  assert.deepEqual(errors(rows), {});
});

test('bodyFor and changed', () => {
  const original = toRows(effective);
  assert.equal(changed(original, original), false);
  const rows = flip(original, 'a');
  assert.equal(changed(rows, original), true);
  assert.deepEqual(bodyFor(rows, 'later'), { roots: [{ root_id: 'a', mount_path: '/work/portal', read_only: true }], apply: 'later' });
});

test('policy lists round trip', () => {
  const spec = { network_policies: [], filesystem_policy: { read_only: ['/etc'], read_write: ['/tmp'] } };
  assert.deepEqual(policyLists(spec), { ro: ['/etc'], rw: ['/tmp'] });
  const next = setPolicyLists(spec, ['/usr'], []);
  assert.deepEqual(next.filesystem_policy, { read_only: ['/usr'], read_write: [] });
  assert.deepEqual(next.network_policies, []);
  assert.deepEqual(policyLists(null), { ro: [], rw: [] });
});
