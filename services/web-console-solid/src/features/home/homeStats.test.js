import test from 'node:test';
import assert from 'node:assert/strict';
import { totals, health, graphFromTree, pathFor } from './homeStats.js';

const tree = {
  kind: 'organization', id: 'o', name: 'Platform', children: [
    { kind: 'data_center', id: 'dc', name: 'Berlin', children: [
      { kind: 'site', id: 's', name: 'main', children: [
        { kind: 'node', id: 'n1', name: 'gpu-01', status: 'online', gpu: true, tunnel_state: 'up', children: [
          { kind: 'sandbox', id: 'b1', name: 'a', status: 'running', allow_all: true, alerts: 2, children: [] },
          { kind: 'sandbox', id: 'b2', name: 'b', status: 'failed', alerts: 0, children: [] },
        ] },
        { kind: 'node', id: 'n2', name: 'cpu-02', status: 'offline', tunnel_state: 'down', children: [] },
        { kind: 'node', id: 'n3', name: 'cpu-03', status: 'online', tunnel_state: 'drifted', children: [] },
      ] },
    ] },
  ],
};

test('totals count every kind', () => {
  assert.deepEqual(totals(tree), { dataCenters: 1, sites: 1, nodes: 3, nodesOnline: 2, sandboxes: 2, running: 1, allowAll: 1, alerts: 2, gpuNodes: 1 });
});

test('totals of nothing are zero', () => {
  assert.equal(totals(null).nodes, 0);
});

test('health lists offline, failed, drifted, allow-all in that order', () => {
  assert.deepEqual(health(tree).map((row) => row.id), ['n2', 'b2', 'n3', 'b1']);
});

test('the graph has four layers and parent-to-child edges', () => {
  const graph = graphFromTree(tree);
  assert.equal(graph.nodes.length, 7);
  assert.ok(!graph.nodes.some((n) => n.kind === 'organization'));
  assert.deepEqual(graph.edges.find((e) => e.to === 'b1'), { from: 'n1', to: 'b1' });
  assert.equal(graph.edges.find((e) => e.to === 'dc'), undefined);
});

test('paths follow the contract', () => {
  assert.equal(pathFor('node', 'x'), '/node/x');
  assert.equal(pathFor('user', 'x'), null);
});
