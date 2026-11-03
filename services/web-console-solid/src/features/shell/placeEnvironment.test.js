import test from 'node:test';
import assert from 'node:assert/strict';
import { placeEnvironment } from './placeEnvironment.js';

const U = (n) => `00000000-0000-0000-0000-0000000000${n}`;

// Two data centers like the appliance: `default` holds the sandboxes, `Berlin` is empty.
const root = {
  kind: 'organization', id: U('00'), name: 'Platform', children: [
    { kind: 'data_center', id: U('02'), name: 'default', children: [
      { kind: 'site', id: U('10'), name: 'main', children: [
        { kind: 'node', id: U('20'), name: 'gpu-01', children: [
          { kind: 'sandbox', id: U('30'), name: 'desktop-01', children: [] },
        ] },
      ] },
      { kind: 'model', id: 'deepseek-r1-7b', name: 'deepseek-r1-7b', children: [] },
    ] },
    { kind: 'data_center', id: U('03'), name: 'Berlin', children: [] },
  ],
};

test('a sandbox link answers the data center that holds the sandbox', () => {
  assert.equal(placeEnvironment(root, { kind: 'box', id: U('30') }), U('02'));
  assert.equal(placeEnvironment(root, { kind: 'admin', id: U('30') }), U('02'));
});

test('a node, a site and a model answer the data center above them', () => {
  assert.equal(placeEnvironment(root, { kind: 'node', id: U('20') }), U('02'));
  assert.equal(placeEnvironment(root, { kind: 'site', id: U('10') }), U('02'));
  assert.equal(placeEnvironment(root, { kind: 'model', id: 'deepseek-r1-7b' }), U('02'));
});

test('a data center answers itself, including an empty one', () => {
  assert.equal(placeEnvironment(root, { kind: 'dc', id: U('02') }), U('02'));
  assert.equal(placeEnvironment(root, { kind: 'dc', id: U('03') }), U('03'));
});

test('places without a tree entity answer null', () => {
  for (const place of [{ kind: 'home' }, { kind: 'menu', id: 'traffic' }, { kind: 'docs', slug: null }, { kind: 'my' }]) {
    assert.equal(placeEnvironment(root, place), null);
  }
});

test('an unknown id answers null, so the page shows its own error', () => {
  assert.equal(placeEnvironment(root, { kind: 'box', id: U('99') }), null);
});

test('without a tree or a place there is nothing to answer', () => {
  assert.equal(placeEnvironment(null, { kind: 'box', id: U('30') }), null);
  assert.equal(placeEnvironment(root, null), null);
});
