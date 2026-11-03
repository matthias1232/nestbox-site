import test from 'node:test';
import assert from 'node:assert/strict';
import { GROUPS, ROUTES, RANK, menuFor } from './navigation.js';

const flat = (role) => menuFor(role).flatMap((group) => group.routes.map((route) => route.id));

test('every route sits in exactly one known group', () => {
  const groups = new Set(GROUPS.map((group) => group.id));
  for (const route of ROUTES) assert.ok(groups.has(route.group), route.id);
  const ids = ROUTES.map((route) => route.id);
  assert.equal(new Set(ids).size, ids.length);
});

test('a viewer does not see admin entries, an admin does', () => {
  assert.ok(!flat('viewer').includes('users'));
  assert.ok(!flat('viewer').includes('providers'));
  assert.ok(flat('admin').includes('users'));
});

test('a group with nothing to show is dropped', () => {
  assert.ok(!menuFor('viewer').some((group) => group.id === 'admin'));
});

test('a member sees their own sandboxes and the menu, not the administration', () => {
  const ids = flat('member');
  assert.ok(ids.includes('my'));
  assert.ok(ids.includes('sandboxes'));
  assert.ok(!ids.includes('users'));
  assert.equal(RANK.member > RANK.viewer, true);
});

test('an unknown role sees only unrestricted entries', () => {
  assert.ok(!flat('nobody').includes('users'));
  assert.ok(flat('nobody').includes('sandboxes'));
});

test('the plan\'s groups exist in order', () => {
  assert.deepEqual(GROUPS.map((group) => group.id), ['overview', 'operate', 'setup', 'security', 'admin', 'help']);
});

test('the new Control Center routes are in the menu', () => {
  const ids = flat('admin');
  for (const id of ['home', 'traffic', 'logs', 'alerts', 'my', 'sandboxes', 'models', 'clusters', 'moves',
    'persistent', 'operations', 'tenants', 'nodes', 'hosting', 'images', 'providers', 'mcp', 'media', 'skills',
    'cudax', 'builder', 'recipes', 'proxies', 'policies', 'privacy', 'master', 'usage', 'users', 'system',
    'cluster', 'docs', 'shortcuts']) {
    assert.ok(ids.includes(id), id);
  }
});

test('a member does not see operator entries but sees the docs', () => {
  const ids = flat('member');
  assert.ok(!ids.includes('system'));
  assert.ok(!ids.includes('hosting'));
  assert.ok(ids.includes('docs'));
});

test('alerts and shortcuts are actions, not pages', () => {
  assert.equal(ROUTES.find((route) => route.id === 'alerts').action, 'dock');
  assert.equal(ROUTES.find((route) => route.id === 'shortcuts').action, 'shortcuts');
});

test('a hidden route never appears in the menu, whatever the role', () => {
  for (const role of ['viewer', 'member', 'operator', 'admin', 'nobody']) {
    assert.ok(!flat(role).includes('live'), role);
  }
  // The page and its path stay, so an old link still opens it.
  assert.ok(ROUTES.some((route) => route.id === 'live'));
  assert.equal(menuFor('admin').flatMap((group) => group.routes).length, ROUTES.length - 1);
});
