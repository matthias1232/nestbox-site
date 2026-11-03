import test from 'node:test';
import assert from 'node:assert/strict';
import {
  nodeFit, candidateNodes, groupNodes, selectFitting, selectBySite, selectByDc, counts, summary,
  bodyFor, nodesProblem, optionsProblem, progressRows, overall, clusterIdOf, reviewLines,
} from './bulkDeploy.js';

const GIB = 1073741824;
const model = { key: 'm1', name: 'M1', runtime: 'cuda', min_vram_bytes: 8 * GIB, recommended_vram_bytes: 16 * GIB };
const tree = {
  kind: 'organization', id: 'o', children: [
    { kind: 'data_center', id: 'd1', name: 'Berlin', children: [
      { kind: 'site', id: 's1', name: 'main', children: [
        { kind: 'node', id: 'n1', name: 'gpu-01', status: 'online', gpu: true, children: [] },
        { kind: 'node', id: 'n2', name: 'gpu-02', status: 'online', gpu: true, children: [] },
        { kind: 'node', id: 'n3', name: 'cpu-01', status: 'online', gpu: false, children: [] },
      ] },
      { kind: 'site', id: 's2', name: 'lab', children: [
        { kind: 'node', id: 'n4', name: 'gpu-04', status: 'offline', gpu: true, children: [] },
      ] },
    ] },
    { kind: 'data_center', id: 'd2', name: 'Zurich', children: [
      { kind: 'site', id: 's3', name: 'rack', children: [
        { kind: 'node', id: 'n5', name: 'gpu-05', status: 'online', gpu: true, children: [] },
      ] },
    ] },
  ],
};
const gpuNode = (id, free) => ({ id, container_gpu: true, status: 'online', budget: { vram_total_bytes: 24 * GIB, vram_free_bytes: free * GIB, ram_free_bytes: 64 * GIB } });
const nodes = [
  { ...gpuNode('n1', 24), name: 'gpu-01' },
  { ...gpuNode('n2', 10), name: 'gpu-02' },
  { id: 'n3', name: 'cpu-01', status: 'online', container_gpu: false, budget: { ram_free_bytes: 64 * GIB } },
  { ...gpuNode('n4', 24), name: 'gpu-04', status: 'offline' },
  { ...gpuNode('n5', 4), name: 'gpu-05' },
];

test('a node with enough free VRAM fits, one above the minimum is tight, below is no', () => {
  const c = candidateNodes(nodes, tree, { model });
  const by = Object.fromEntries(c.map((r) => [r.name, r.fit]));
  assert.equal(by['gpu-01'], 'fits');
  assert.equal(by['gpu-02'], 'tight');
  assert.equal(by['gpu-05'], 'no');
});

test('a GPU model is refused on a node without a GPU and on an offline node', () => {
  const c = candidateNodes(nodes, tree, { model });
  const by = Object.fromEntries(c.map((r) => [r.name, r]));
  assert.equal(by['cpu-01'].fit, 'no');
  assert.equal(by['gpu-04'].fit, 'no');
  assert.match(by['gpu-04'].reason, /offline/);
});

test('a hybrid model runs on a CPU node using RAM', () => {
  const hybrid = { runtime: 'hybrid', min_ram_bytes: 8 * GIB };
  assert.equal(nodeFit(hybrid, { online: true, gpu: false, budget: { ram_free_bytes: 16 * GIB } }).fit, 'fits');
});

test('filters narrow by text, site and fitting', () => {
  assert.equal(candidateNodes(nodes, tree, { model, text: 'cpu' }).length, 1);
  assert.equal(candidateNodes(nodes, tree, { model, site: 's1' }).length, 3);
  assert.equal(candidateNodes(nodes, tree, { model, onlyFitting: true }).length, 2);
});

test('selection helpers pick fitting nodes only', () => {
  const c = candidateNodes(nodes, tree, { model });
  assert.deepEqual([...selectFitting(c)].sort(), ['n1', 'n2']);
  assert.deepEqual([...selectBySite(c, 's1')].sort(), ['n1', 'n2']);
  assert.deepEqual([...selectByDc(c, 'd2')], []);
});

test('groupNodes nests site under data center', () => {
  const g = groupNodes(candidateNodes(nodes, tree, { model }));
  assert.equal(g.length, 2);
  assert.equal(g[0].dcName, 'Berlin');
  assert.equal(g[0].sites.length, 2);
});

test('summary counts selected, tight and skipped', () => {
  const c = candidateNodes(nodes, tree, { model });
  const sel = new Set(['n1', 'n2', 'n3', 'n5']);
  assert.deepEqual(counts(c, sel), { selected: 4, fits: 1, tight: 1, skipped: 2 });
  assert.equal(summary(c, sel), '4 selected · 1 tight · 2 skipped');
  assert.equal(summary(c, new Set(['n1'])), '1 selected');
});

test('bodyFor sends node names and the cluster only when named', () => {
  const cands = candidateNodes(nodes, tree, { model });
  const base = { cands, selected: new Set(['n1', 'n2']), force: false, makeCluster: true, clusterName: ' x4 ', strategy: 'failover' };
  assert.deepEqual(bodyFor(base), {
    nodes: ['gpu-01', 'gpu-02'], site_id: null, environment_wide: false, force: false,
    make_cluster: { name: 'x4', strategy: 'failover' },
  });
  assert.equal('make_cluster' in bodyFor({ ...base, clusterName: '' }), false);
  assert.equal('make_cluster' in bodyFor({ ...base, makeCluster: false }), false);
});

test('problems block an empty or all-skipped selection and an unnamed cluster', () => {
  const c = candidateNodes(nodes, tree, { model });
  assert.ok(nodesProblem(c, new Set()));
  assert.ok(nodesProblem(c, new Set(['n3'])));
  assert.equal(nodesProblem(c, new Set(['n1'])), null);
  assert.ok(optionsProblem({ makeCluster: true, clusterName: ' ' }));
  assert.equal(optionsProblem({ makeCluster: false }), null);
});

test('progress rows follow install state and overall ignores skipped', () => {
  const targets = [{ node: 'gpu-01' }, { node: 'gpu-02' }, { node: 'cpu-01', skipped: 'no_gpu' }, { node: 'gpu-05' }];
  const installs = [{ node_name: 'gpu-01', state: 'installed' }, { node_name: 'gpu-02', state: 'downloading' }];
  const rows = progressRows(targets, installs, [{ message: 'gpu-05 queued' }]);
  assert.deepEqual(rows.map((r) => r.state), ['installed', 'installing', 'skipped', 'queued']);
  assert.equal(rows[3].tail, 'gpu-05 queued');
  const o = overall(rows);
  assert.deepEqual(o, { done: 1, failed: 0, total: 3, percent: 33, finished: false });
  assert.equal(overall(progressRows([{ node: 'a' }], [{ node_name: 'a', state: 'failed', error: 'x' }], [])).finished, true);
});

test('clusterIdOf reads the job result', () => {
  assert.equal(clusterIdOf({ result: { cluster_id: 'c1' } }), 'c1');
  assert.equal(clusterIdOf({ result: null }), null);
});

test('reviewLines names the nodes, the cluster and the grant', () => {
  const cands = candidateNodes(nodes, tree, { model });
  const lines = reviewLines({ modelKey: 'm1', cands, selected: new Set(['n1']), force: true, makeCluster: true, clusterName: 'c', strategy: 'round_robin', grant: true }, 'M1');
  assert.equal(lines.length, 4);
  assert.match(lines[0], /gpu-01/);
});
