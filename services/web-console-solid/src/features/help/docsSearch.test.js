import test from 'node:test';
import assert from 'node:assert/strict';
import { search, groupByFirstTag, splitAnchor } from './docsSearch.js';

const index = [
  { slug: 'deploy-on-many-nodes', title: 'Deploy one model on twenty nodes', tags: ['models', 'bulk'], summary: 'Roll a model out.', headings: [{ id: 'choose-nodes', text: 'Choose nodes' }] },
  { slug: 'policies', title: 'Policies', tags: ['security'], summary: 'Rules for traffic.', headings: [{ id: 'draft-a-rule', text: 'Draft a rule' }] },
];

test('a title match ranks above a heading match', () => {
  const hits = search(index, 'nodes');
  assert.equal(hits[0].slug, 'deploy-on-many-nodes');
  assert.equal(hits[0].heading, undefined);
  assert.equal(hits[1].headingId, 'choose-nodes');
});

test('every word must match', () => {
  assert.deepEqual(search(index, 'policies banana'), []);
});

test('an empty query finds nothing', () => {
  assert.deepEqual(search(index, '  '), []);
});

test('tags and summary count', () => {
  assert.equal(search(index, 'bulk')[0].slug, 'deploy-on-many-nodes');
  assert.equal(search(index, 'traffic')[0].slug, 'policies');
});

test('documents group by first tag', () => {
  assert.deepEqual(groupByFirstTag(index).map((g) => g.group), ['models', 'security']);
});

test('an anchor splits into slug and heading', () => {
  assert.deepEqual(splitAnchor('a#b'), { slug: 'a', heading: 'b' });
  assert.deepEqual(splitAnchor('a'), { slug: 'a', heading: null });
  assert.deepEqual(splitAnchor(null), { slug: null, heading: null });
});
