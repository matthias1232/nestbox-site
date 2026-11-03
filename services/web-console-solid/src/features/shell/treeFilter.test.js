import test from 'node:test';
import assert from 'node:assert/strict';
import { filterTree, treePathFor, pathFor, ancestorIds, expandableIds } from './treeFilter.js';

const U = (n) => `00000000-0000-0000-0000-00000000000${n}`;
const root = {
  kind: 'organization', id: U(0), name: 'Platform', children: [
    { kind: 'data_center', id: U(1), name: 'Berlin', children: [
      { kind: 'site', id: U(2), name: 'main', children: [
        { kind: 'node', id: U(3), name: 'gpu-07', children: [
          { kind: 'sandbox', id: U(4), name: 'research-01', children: [] },
          { kind: 'sandbox', id: U(5), name: 'build-02', children: [] },
        ] },
      ] },
    ] },
    { kind: 'data_center', id: U(6), name: 'Zurich', children: [] },
  ],
};

test('a filter keeps the ancestors of a match and drops the rest', () => {
  const out = filterTree(root, 'RESEARCH');
  assert.deepEqual(out.children.map((c) => c.name), ['Berlin']);
  const node = out.children[0].children[0].children[0];
  assert.deepEqual(node.children.map((c) => c.name), ['research-01']);
});

test('an empty filter returns the tree unchanged and no match leaves only the root', () => {
  assert.equal(filterTree(root, '  '), root);
  assert.deepEqual(filterTree(root, 'nothing-here').children, []);
});

test('a matching node keeps all its children', () => {
  const out = filterTree(root, 'gpu');
  assert.equal(out.children[0].children[0].children[0].children.length, 2);
});

test('pathFor builds the console address for each kind', () => {
  assert.equal(pathFor({ kind: 'organization', id: 'x' }), '/');
  assert.equal(pathFor({ kind: 'data_center', id: U(1) }), `/dc/${U(1)}`);
  assert.equal(pathFor({ kind: 'site', id: U(2) }), `/site/${U(2)}`);
  assert.equal(pathFor({ kind: 'node', id: U(3) }), `/node/${U(3)}`);
  assert.equal(pathFor({ kind: 'sandbox', id: U(4) }), `/sandboxes/${U(4)}`);
  assert.equal(pathFor({ kind: 'user', id: 'ada' }), '/sandboxes?owner=ada');
  assert.equal(pathFor({ kind: 'model', id: 'deepseek-r1-7b' }), '/models/deepseek-r1-7b');
  assert.equal(pathFor({ kind: 'install', id: 'i', node_id: U(3) }), `/node/${U(3)}`);
  assert.equal(pathFor({ kind: 'cluster', id: U(7) }), `/clusters/${U(7)}`);
});

test('treePathFor lists the trail from the root, accepting route kinds', () => {
  const trail = treePathFor(root, 'admin', U(4));
  assert.deepEqual(trail.map((t) => t.label), ['Platform', 'Berlin', 'main', 'gpu-07', 'research-01']);
  assert.equal(trail[1].path, `/dc/${U(1)}`);
  assert.deepEqual(treePathFor(root, 'dc', 'missing'), []);
});

test('ancestorIds and expandableIds support opening the way to a place', () => {
  assert.deepEqual(ancestorIds(root, 'node', U(3)), [U(0), U(1), U(2)]);
  assert.ok(expandableIds(root).has(U(3)));
  assert.ok(!expandableIds(root).has(U(6)));
});
