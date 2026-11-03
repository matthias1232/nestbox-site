import test from 'node:test';
import assert from 'node:assert/strict';
import { applyFilters, counts, targetOptions, nodeBySandbox, kindOf, decisionOf } from './trafficFilters.js';

const rows = [
  { id: 1, kind: 'network', channel: 'proxy', decision: 'allow', sandboxId: 'a' },
  { id: 2, kind: 'network', channel: 'proxy', decision: 'deny', sandboxId: 'a' },
  { id: 3, kind: 'dns', channel: 'dns', decision: 'deny', sandboxId: 'b' },
  { id: 4, kind: 'mcp', channel: 'mcp', decision: 'audit', sandboxId: 'b' },
  { id: 5, kind: 'network', channel: 'inference', decision: 'allow', sandboxId: 'b' },
  { id: 6, kind: 'network', channel: 'proxy', decision: 'allow', would_deny: true, sandboxId: 'a' },
];
const tree = {
  kind: 'organization', name: 'Org', children: [
    { kind: 'node', id: 'n1', name: 'gpu-01', children: [{ kind: 'sandbox', id: 'a', name: 'alpha', children: [] }] },
    { kind: 'node', id: 'n2', name: 'gpu-02', children: [{ kind: 'sandbox', id: 'b', name: 'beta', children: [] }] },
  ],
};

test('rows are sorted into kind and decision chips', () => {
  assert.equal(kindOf(rows[2]), 'dns');
  assert.equal(kindOf(rows[4]), 'inference');
  assert.equal(decisionOf(rows[3]), 'would_deny');
  assert.equal(decisionOf(rows[5]), 'would_deny');
  assert.equal(decisionOf(rows[1]), 'refused');
});

test('the three filters combine', () => {
  const got = applyFilters(rows, { kind: 'proxy', decision: 'refused', target: 'sandbox:a' });
  assert.deepEqual(got.map((r) => r.id), [2]);
  assert.equal(applyFilters(rows, {}).length, 6);
});

test('a node target uses the sandbox-to-node map', () => {
  const map = nodeBySandbox(tree);
  assert.deepEqual(applyFilters(rows, { target: 'node:gpu-02' }, map).map((r) => r.id), [3, 4, 5]);
});

test('counts describe what each chip would show', () => {
  const c = counts(rows, { kind: 'all', decision: 'refused', target: '' });
  assert.equal(c.kind.all, 2);
  assert.equal(c.kind.dns, 1);
  assert.equal(c.decision.refused, 2);
  assert.equal(c.decision.allowed, 2);
});

test('target options list nodes then sandboxes', () => {
  const options = targetOptions(tree);
  assert.deepEqual(options.map((o) => o.value), ['node:gpu-01', 'node:gpu-02', 'sandbox:a', 'sandbox:b']);
  assert.deepEqual(targetOptions(null), []);
});
