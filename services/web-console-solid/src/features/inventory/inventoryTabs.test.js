import test from 'node:test';
import assert from 'node:assert/strict';
import { activeTab, ago, countBelow, findNode, gib, nodesBelow, pairText, stamp, tabFromSearch, tabPath, tabsFor, toGib, worstStatus } from './inventoryTabs.js';

const ids = (kind, role) => tabsFor(kind, role).map((t) => t.id);

test('a member sees Summary and Sandboxes only', () => {
  assert.deepEqual(ids('node', 'member'), ['summary', 'sandboxes']);
  assert.deepEqual(ids('dc', 'member'), ['summary', 'sandboxes']);
  assert.deepEqual(ids('site', 'viewer'), ['summary', 'sandboxes']);
});

test('an operator has no Permissions tab, an admin has', () => {
  assert.ok(!ids('dc', 'operator').includes('permissions'));
  assert.ok(ids('dc', 'admin').includes('permissions'));
  assert.deepEqual(ids('node', 'admin'), ['summary', 'sandboxes', 'models', 'images', 'traffic', 'logs']);
});

test('an unknown tab falls back to Summary', () => {
  assert.equal(activeTab('node', 'admin', 'logs'), 'logs');
  assert.equal(activeTab('node', 'member', 'logs'), 'summary');
  assert.equal(activeTab('node', 'admin', ''), 'summary');
});

test('the tab lives in the query string', () => {
  assert.equal(tabFromSearch('?tab=models'), 'models');
  assert.equal(tabFromSearch(''), '');
  assert.equal(tabPath('/node/1', 'summary'), '/node/1');
  assert.equal(tabPath('/node/1', 'logs'), '/node/1?tab=logs');
});

test('sizes use binary units', () => {
  assert.equal(gib(1073741824), '1.0 GiB');
  assert.equal(gib(512), '512 B');
  assert.equal(gib(null), '–');
  assert.equal(toGib(2147483648), 2);
});

test('a gauge reads used over total, or a dash when the machine reports no total', () => {
  assert.equal(pairText(41 * 1073741824, 64 * 1073741824), '41 / 64 GiB');
  assert.equal(pairText(0, null), '–');
  assert.equal(pairText(0, 0), '–');
});

test('age reads in words', () => {
  const now = Date.parse('2026-11-03T12:00:00Z');
  assert.equal(ago('2026-11-03T11:57:00Z', now), '3 min ago');
  assert.equal(ago('nope', now), '–');
});

test('a timestamp reads as the local date and minute, not a raw ISO string', () => {
  const at = new Date(2026, 9, 27, 1, 5);
  assert.equal(stamp(at.toISOString()), '2026-10-27 01:05');
  assert.equal(stamp('2026-10-27T01:00:25.368741Z').length, 16);
  assert.equal(stamp(''), '–');
  assert.equal(stamp('nope'), '–');
});

const tree = {
  kind: 'organization', id: 'o', children: [
    { kind: 'data_center', id: 'd', children: [
      { kind: 'site', id: 's', children: [
        { kind: 'node', id: 'n1', status: 'online', children: [{ kind: 'sandbox', id: 'a', status: 'running', children: [] }, { kind: 'sandbox', id: 'b', status: 'stopped', children: [] }] },
        { kind: 'node', id: 'n2', status: 'offline', children: [] },
      ] },
    ] },
  ],
};

test('the tree helpers count and find', () => {
  assert.deepEqual(countBelow(tree), { sandboxes: 2, running: 1, nodes: 2, online: 1, sites: 1 });
  assert.equal(findNode(tree, 'dc', 'd').kind, 'data_center');
  assert.equal(findNode(tree, 'node', 'zz'), null);
  assert.equal(nodesBelow(findNode(tree, 'site', 's')).length, 2);
});

test('the worst status wins', () => {
  assert.equal(worstStatus(['ok', 'offline']), 'bad');
  assert.equal(worstStatus(['ok', 'degraded']), 'warn');
  assert.equal(worstStatus([]), 'ok');
});
