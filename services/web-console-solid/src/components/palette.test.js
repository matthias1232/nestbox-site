import test from 'node:test';
import assert from 'node:assert/strict';
import {
  applyFilters,
  createLatest,
  flatten,
  matchDocs,
  matchMenu,
  menuEntries,
  mergeResults,
  moveIndex,
  recentRecord,
  searchParams,
  serverItems,
} from './palette.js';

const menu = [
  { label: 'Operate', routes: [{ id: 'models', label: 'Models' }, { id: 'sandboxes', label: 'Sandboxes' }] },
  { label: 'Overview', routes: [{ id: 'alerts', label: 'Alerts', action: 'dock' }, { id: 'traffic', label: 'Live traffic' }] },
];
const pathFor = (id) => `/${id}`;

test('menu entries skip routes that only run an action', () => {
  const entries = menuEntries(menu, pathFor);
  assert.deepEqual(entries.map((e) => e.name), ['Models', 'Sandboxes', 'Live traffic']);
  assert.equal(entries[0].path, '/models');
});

test('menu matching is by label and puts prefix matches first', () => {
  const entries = menuEntries(menu, pathFor);
  assert.deepEqual(matchMenu(entries, 'tra').map((e) => e.name), ['Live traffic']);
  assert.deepEqual(matchMenu(entries, 'S').map((e) => e.name), ['Sandboxes', 'Models']);
  assert.deepEqual(matchMenu(entries, ''), []);
});

const index = [
  { slug: 'deploy', title: 'Deploy a model', tags: ['models'], summary: 's1', headings: [{ id: 'choose-nodes', text: 'Choose nodes' }] },
  { slug: 'nodes', title: 'Nodes', tags: ['gpu'], summary: 's2', headings: [{ id: 'model-fit', text: 'Model fit' }] },
  { slug: 'other', title: 'Other', tags: [], summary: '', headings: [] },
];

test('docs score title 3, tag 2, heading 1 and link to a heading when only it matched', () => {
  const found = matchDocs(index, 'model');
  assert.deepEqual(found.map((d) => d.name), ['Deploy a model', 'Nodes']);
  // "Deploy a model": title 3 + tag 2 = 5 beats "Nodes": heading 1.
  assert.equal(found[0].path, '/docs/deploy');
  assert.equal(found[1].path, '/docs/nodes#model-fit');
  assert.equal(found[1].subtitle, 'Model fit');
  assert.deepEqual(matchDocs(index, 'zzz'), []);
});

test('server items get a kind-prefixed id', () => {
  const [item] = serverItems([{ kind: 'node', id: 'n1', name: 'gpu-07', subtitle: 'Berlin', status: 'online', environment_id: 'e1', path: '/node/n1' }]);
  assert.equal(item.id, 'node:n1');
  assert.equal(item.path, '/node/n1');
});

const server = serverItems([
  { kind: 'user', id: 'u', name: 'ada', status: null, environment_id: 'e1', path: '/u' },
  { kind: 'sandbox', id: 's', name: 'box', status: 'running', environment_id: 'e1', path: '/s' },
  { kind: 'site', id: 'p', name: 'main', environment_id: 'e2', path: '/p' },
  { kind: 'data_center', id: 'd', name: 'Berlin', environment_id: 'e1', path: '/d' },
  { kind: 'node', id: 'n', name: 'gpu', status: 'offline', environment_id: 'e2', path: '/n' },
]);
const docs = matchDocs(index, 'nodes');
const menuHits = matchMenu(menuEntries(menu, pathFor), 'models');

test('groups follow the fixed order and places share one group', () => {
  const groups = mergeResults(server, menuHits, docs, [], 'a');
  assert.deepEqual(groups.map((g) => g.id), ['sandbox', 'node', 'user', 'place', 'doc', 'menu']);
  assert.equal(groups.find((g) => g.id === 'place').items.length, 2);
});

test('an empty query shows only the recent items', () => {
  const recent = [recentRecord(server[0])];
  const groups = mergeResults(server, menuHits, docs, recent, '  ');
  assert.deepEqual(groups.map((g) => g.id), ['recent']);
  assert.equal(groups[0].items[0].subtitle, '');
  assert.deepEqual(mergeResults(server, menuHits, docs, [], ''), []);
});

test('filters keep only matching kind, data center and status', () => {
  const groups = mergeResults(server, menuHits, docs, [], 'a');
  assert.deepEqual(flatten(applyFilters(groups, { kind: 'node' })).map((i) => i.name), ['gpu']);
  assert.deepEqual(flatten(applyFilters(groups, { environment: 'e2' })).map((i) => i.name), ['gpu', 'main']);
  assert.deepEqual(flatten(applyFilters(groups, { status: 'running' })).map((i) => i.name), ['box']);
  assert.equal(applyFilters(groups, {}).length, groups.length);
  assert.deepEqual(applyFilters(groups, { kind: 'job' }), []);
});

test('arrow movement wraps and handles an empty list', () => {
  assert.equal(moveIndex(0, -1, 3), 2);
  assert.equal(moveIndex(2, 1, 3), 0);
  assert.equal(moveIndex(0, 1, 3), 1);
  assert.equal(moveIndex(0, 1, 0), -1);
});

test('search params omit empty chips and unknown kinds', () => {
  assert.deepEqual(searchParams(' box ', {}), { q: 'box', limit: 20 });
  assert.deepEqual(searchParams('x', { kind: 'node', environment: 'e1', status: 'online' }), {
    q: 'x', limit: 20, kind: 'node', environment: 'e1', status: 'online',
  });
  assert.equal(searchParams('x', { kind: 'docs' }).kind, undefined);
});

test('only the newest request is current', () => {
  const latest = createLatest();
  const first = latest.begin();
  const second = latest.begin();
  assert.equal(latest.isCurrent(first), false);
  assert.equal(latest.isCurrent(second), true);
});
