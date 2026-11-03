import test from 'node:test';
import assert from 'node:assert/strict';
import { stamp, mergeByTime, sourceLabel, sandboxIdsOf } from './logLoaders.js';

test('stamp puts the sandbox name in front of the source', () => {
  assert.equal(stamp('2026-11-03T10:00:00Z [proxy] refused x', 'box'), '2026-11-03T10:00:00Z [box proxy] refused x');
});

test('stamp keeps a line without a time', () => {
  assert.equal(stamp('plain text', 'box'), 'plain text');
});

test('mergeByTime orders lines of several groups by time', () => {
  const text = mergeByTime([['2026-11-03T10:00:02Z b'], ['2026-11-03T10:00:01Z a', '2026-11-03T10:00:03Z c']]);
  assert.equal(text, '2026-11-03T10:00:01Z a\n2026-11-03T10:00:02Z b\n2026-11-03T10:00:03Z c');
});

test('mergeByTime keeps only the last lines', () => {
  assert.equal(mergeByTime([['a', 'b', 'c']], 2), 'b\nc');
});

test('sourceLabel names each kind of source', () => {
  assert.equal(sourceLabel('all'), 'All sandboxes');
  assert.equal(sourceLabel('sandbox:abc', { sandbox: { abc: 'research' } }), 'Sandbox research');
  assert.equal(sourceLabel('node:n1', { node: { n1: 'gpu-07' } }), 'Node gpu-07');
  assert.equal(sourceLabel('system:runner'), 'Platform: runner');
});

test('sandboxIdsOf returns ids only for sandbox based sources', () => {
  assert.deepEqual(sandboxIdsOf('all', ['a', 'b']), ['a', 'b']);
  assert.deepEqual(sandboxIdsOf('sandbox:x'), ['x']);
  assert.equal(sandboxIdsOf('system:runner'), null);
});
