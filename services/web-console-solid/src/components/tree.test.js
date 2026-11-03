import test from 'node:test';
import assert from 'node:assert/strict';
import { visibleRows, nextIndex, toggleAction, filterTree } from './tree.js';

const root = {
  id: 'o', name: 'Org', children: [
    { id: 'd', name: 'Berlin', children: [{ id: 's', name: 'main', children: [] }] },
    { id: 'e', name: 'Paris', children: [] },
  ],
};

test('a collapsed root shows only itself', () => {
  assert.deepEqual(visibleRows(root, new Set()).map((r) => r.node.id), ['o']);
});

test('expanded nodes show their children with depth', () => {
  const rows = visibleRows(root, new Set(['o', 'd']));
  assert.deepEqual(rows.map((r) => [r.node.id, r.depth, r.hasChildren]), [['o', 0, true], ['d', 1, true], ['s', 2, false], ['e', 1, false]]);
});

test('arrow keys step and clamp', () => {
  const rows = visibleRows(root, new Set(['o']));
  assert.equal(nextIndex(rows, 0, 'ArrowDown'), 1);
  assert.equal(nextIndex(rows, 2, 'ArrowDown'), 2);
  assert.equal(nextIndex(rows, 0, 'ArrowUp'), 0);
  assert.equal(nextIndex(rows, 1, 'End'), 2);
});

test('left on a leaf goes to the parent, right on an open row to the child', () => {
  const expanded = new Set(['o', 'd']);
  const rows = visibleRows(root, expanded);
  assert.equal(nextIndex(rows, 2, 'ArrowLeft', expanded), 1);
  assert.equal(nextIndex(rows, 1, 'ArrowRight', expanded), 2);
  assert.equal(nextIndex(rows, 1, 'ArrowLeft', expanded), 1);
});

test('toggleAction expands closed and collapses open rows', () => {
  const rows = visibleRows(root, new Set(['o']));
  assert.equal(toggleAction(rows, 1, 'ArrowRight', new Set(['o'])), 'expand');
  assert.equal(toggleAction(rows, 0, 'ArrowLeft', new Set(['o'])), 'collapse');
  assert.equal(toggleAction(rows, 2, 'ArrowRight', new Set(['o'])), null);
});

test('filterTree keeps the path to a match', () => {
  const kept = filterTree(root, 'main');
  assert.deepEqual(visibleRows(kept, new Set(['o', 'd', 's'])).map((r) => r.node.id), ['o', 'd', 's']);
  assert.equal(filterTree(root, 'zzz'), null);
});
