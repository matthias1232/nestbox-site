import test from 'node:test';
import assert from 'node:assert/strict';
import {
  STORAGE_PREFIX, emptyDataCenterNotice, environmentsWithNodes, firstUsable, pickEnvironment, storageKey,
} from './environments.js';

const CENTERS = [
  { id: 'dc-berlin', name: 'Berlin', role: 'admin' },
  { id: 'dc-default', name: 'default', role: 'admin' },
];

test('a remembered data center wins while the server still lists it', () => {
  assert.equal(pickEnvironment(CENTERS, 'dc-default', 'dc-berlin'), 'dc-default');
});

test('without a remembered choice the server current one opens', () => {
  assert.equal(pickEnvironment(CENTERS, null, 'dc-berlin'), 'dc-berlin');
});

test('a remembered data center the user may no longer see is ignored', () => {
  assert.equal(pickEnvironment(CENTERS, 'dc-gone', 'dc-default'), 'dc-default');
  assert.equal(pickEnvironment(CENTERS, 'dc-gone', 'dc-also-gone'), null);
  assert.equal(pickEnvironment([], 'dc-default', 'dc-default'), null);
});

test('the storage key names the user, and nothing else', () => {
  assert.equal(STORAGE_PREFIX, 'nestbox:environment:');
  assert.equal(storageKey('user-1'), 'nestbox:environment:user-1');
  assert.equal(storageKey(null), null);
});

// The shape `GET /api/v1/organizations` answers on the box: the node count lives here, not in
// `/api/v1/environments` (U02).
const ORGANIZATIONS = [
  {
    id: 'org-platform', name: 'Default Organization', kind: 'platform',
    environments: [
      { id: 'dc-berlin', name: 'Berlin', nodes: 0, sandboxes: 0, sites: 1, slug: 'berlin' },
      { id: 'dc-default', name: 'default', nodes: 1, sandboxes: 6, sites: 1, slug: 'default' },
    ],
  },
];

test('organizations flatten to data centers with their node counts', () => {
  assert.deepEqual(environmentsWithNodes(ORGANIZATIONS), [
    { id: 'dc-berlin', name: 'Berlin', nodes: 0 },
    { id: 'dc-default', name: 'default', nodes: 1 },
  ]);
});

test('an entry without an id, without a node count, or without a list is handled', () => {
  assert.deepEqual(environmentsWithNodes([
    { environments: [{ id: 'dc-a', name: 'A' }, { name: 'nameless' }, null] },
    { name: 'no environments here' },
    null,
  ]), [{ id: 'dc-a', name: 'A', nodes: 0 }]);
  assert.deepEqual(environmentsWithNodes(null), []);
  assert.deepEqual(environmentsWithNodes({ items: ORGANIZATIONS }), []);
});

test('a data center with nodes keeps the console where it is', () => {
  const counted = environmentsWithNodes(ORGANIZATIONS);
  assert.equal(firstUsable(counted, 'dc-default'), 'dc-default');
  assert.equal(firstUsable([
    { id: 'dc-default', name: 'default', nodes: 1 },
    { id: 'dc-berlin', name: 'Berlin', nodes: 4 },
  ], 'dc-berlin'), 'dc-berlin');
});

test('an empty current data center gives way to the first one with nodes', () => {
  const counted = environmentsWithNodes(ORGANIZATIONS);
  assert.equal(firstUsable(counted, 'dc-berlin'), 'dc-default');
  // A current one the list does not name cannot be judged, so the first usable one wins too.
  assert.equal(firstUsable(counted, 'dc-gone'), 'dc-default');
  assert.equal(firstUsable(counted, null), 'dc-default');
});

test('nothing usable is null, and a broken list never throws', () => {
  assert.equal(firstUsable([{ id: 'dc-berlin', nodes: 0 }], 'dc-berlin'), null);
  assert.equal(firstUsable([], 'dc-berlin'), null);
  assert.equal(firstUsable(null, 'dc-berlin'), null);
  assert.equal(firstUsable([null, { nodes: 3 }, { id: '', nodes: 3 }], 'dc-berlin'), null);
});

const TREE = {
  kind: 'organization', id: 'org-1', name: 'Acme', counts: { nodes: 3 }, children: [
    { kind: 'data_center', id: 'dc-berlin', name: 'Berlin', counts: { nodes: 0 }, children: [] },
    {
      kind: 'organization', id: 'org-2', name: 'Platform', children: [
        { kind: 'data_center', id: 'dc-default', name: 'default', counts: { nodes: 3 }, children: [] },
      ],
    },
  ],
};

test('an empty data center names the one with nodes', () => {
  assert.deepEqual(emptyDataCenterNotice(TREE, 'dc-berlin'), {
    currentName: 'Berlin',
    targetId: 'dc-default',
    targetName: 'default',
  });
});

test('a data center with nodes says nothing, and neither does a tree without one', () => {
  assert.equal(emptyDataCenterNotice(TREE, 'dc-default'), null);
  assert.equal(emptyDataCenterNotice({ kind: 'data_center', id: 'dc-solo', name: 'Solo', counts: { nodes: 0 }, children: [] }, 'dc-solo'), null);
  assert.equal(emptyDataCenterNotice(null, 'dc-berlin'), null);
  assert.equal(emptyDataCenterNotice(TREE, null), null);
});
