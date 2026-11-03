import test from 'node:test';
import assert from 'node:assert/strict';
import { parsePath, paths, boxPath, terminalPath } from './routing.js';

const ID = '11111111-2222-3333-4444-555555555555';

test('the root and anything unknown are the home page', () => {
  assert.deepEqual(parsePath('/'), { kind: 'home' });
  assert.deepEqual(parsePath('/nonsense'), { kind: 'home' });
  assert.deepEqual(parsePath('/box/xyz'), { kind: 'home' });
  assert.deepEqual(parsePath(`/box/${ID}/terminal/bad tab`), { kind: 'home' });
});

test('data center, site and node pages name their id', () => {
  assert.deepEqual(parsePath(`/dc/${ID}`), { kind: 'dc', id: ID });
  assert.deepEqual(parsePath(`/site/${ID}`), { kind: 'site', id: ID });
  assert.deepEqual(parsePath(`/node/${ID}`), { kind: 'node', id: ID });
});

test('the Admin Center reads the tab query', () => {
  assert.deepEqual(parsePath(`/sandboxes/${ID}`), { kind: 'admin', id: ID, tab: null });
  assert.deepEqual(parsePath(`/sandboxes/${ID}`, '?tab=policies'), { kind: 'admin', id: ID, tab: 'policies' });
});

test('the workbox and its pop-outs', () => {
  assert.deepEqual(parsePath(boxPath(ID)), { kind: 'box', id: ID });
  assert.deepEqual(parsePath(terminalPath(ID, 't-1')), { kind: 'terminal', id: ID, tab: 't-1' });
  assert.deepEqual(parsePath(`/box/${ID}/desktop`), { kind: 'desktop', id: ID });
  assert.equal(parsePath('/my').kind, 'my');
});

test('model, cluster and docs addresses', () => {
  assert.deepEqual(parsePath('/models/deepseek-r1-7b'), { kind: 'model', id: 'deepseek-r1-7b' });
  assert.deepEqual(parsePath(`/clusters/${ID}`), { kind: 'cluster', id: ID });
  assert.deepEqual(parsePath('/docs'), { kind: 'docs', slug: null });
  assert.deepEqual(parsePath('/docs/deploy-on-many-nodes'), { kind: 'docs', slug: 'deploy-on-many-nodes' });
});

test('log and traffic pop-outs', () => {
  assert.deepEqual(parsePath(`/pop/logs/${ID}`), { kind: 'poplogs', id: ID });
  assert.deepEqual(parsePath('/pop/logs/all'), { kind: 'poplogs', id: 'all' });
  assert.deepEqual(parsePath('/pop/traffic'), { kind: 'poptraffic' });
});

test('a menu id is a menu page, the bare list of sandboxes included', () => {
  assert.deepEqual(parsePath('/traffic'), { kind: 'menu', id: 'traffic' });
  assert.deepEqual(parsePath('/policies'), { kind: 'menu', id: 'policies' });
  assert.deepEqual(parsePath('/nodes'), { kind: 'menu', id: 'nodes' });
  assert.deepEqual(parsePath('/sandboxes'), { kind: 'menu', id: 'sandboxes' });
});

test('every path helper round-trips through parsePath', () => {
  assert.deepEqual(parsePath(paths.home()), { kind: 'home' });
  assert.deepEqual(parsePath(paths.dc(ID)), { kind: 'dc', id: ID });
  assert.deepEqual(parsePath(paths.site(ID)), { kind: 'site', id: ID });
  assert.deepEqual(parsePath(paths.node(ID)), { kind: 'node', id: ID });
  assert.deepEqual(parsePath(paths.sandbox(ID)), { kind: 'admin', id: ID, tab: null });
  assert.deepEqual(parsePath(paths.box(ID)), { kind: 'box', id: ID });
  assert.deepEqual(parsePath(paths.my()), { kind: 'my' });
  assert.deepEqual(parsePath(paths.model('qwen3-1.7b')), { kind: 'model', id: 'qwen3-1.7b' });
  assert.deepEqual(parsePath(paths.cluster(ID)), { kind: 'cluster', id: ID });
  assert.deepEqual(parsePath(paths.docs('x')), { kind: 'docs', slug: 'x' });
  assert.deepEqual(parsePath(paths.docs()), { kind: 'docs', slug: null });
  assert.deepEqual(parsePath(paths.popLogs(ID)), { kind: 'poplogs', id: ID });
  assert.deepEqual(parsePath(paths.popLogs()), { kind: 'poplogs', id: 'all' });
  assert.deepEqual(parsePath(paths.popTraffic()), { kind: 'poptraffic' });
  assert.deepEqual(parsePath(paths.menu('traffic')), { kind: 'menu', id: 'traffic' });
  assert.deepEqual(parsePath(paths.menu('home')), { kind: 'home' });
  assert.equal(paths.sandbox(ID, 'policies'), `/sandboxes/${ID}?tab=policies`);
});
