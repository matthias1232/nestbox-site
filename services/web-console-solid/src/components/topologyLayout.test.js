import test from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULTS, layout, edgePath, thickness, fit, zoomAt, toneOf, fromCluster, fromStack } from './topologyLayout.js';

const node = (id, layer, label = id) => ({ id, layer, label });

test('two layers make two columns', () => {
  const out = layout([node('a', 'x'), node('b', 'y')], [{ from: 'a', to: 'b' }], ['x', 'y']);
  assert.equal(out.positions.get('b').x - out.positions.get('a').x, 220);
});

test('a child sits on the row of its parent', () => {
  const nodes = [node('p1', 'x', 'a'), node('p2', 'x', 'b'), node('c2', 'y'), node('c1', 'y')];
  const out = layout(nodes, [{ from: 'p1', to: 'c1' }, { from: 'p2', to: 'c2' }], ['x', 'y']);
  assert.equal(out.positions.get('c1').y, out.positions.get('p1').y);
  assert.equal(out.positions.get('c2').y, out.positions.get('p2').y);
});

test('rows in a column never overlap', () => {
  const nodes = [node('p', 'x'), node('c1', 'y', 'a'), node('c2', 'y', 'b')];
  const out = layout(nodes, [{ from: 'p', to: 'c1' }, { from: 'p', to: 'c2' }], ['x', 'y']);
  assert.ok(out.positions.get('c2').y - out.positions.get('c1').y >= 52);
});

test('fit keeps the aspect ratio and contains the drawing', () => {
  const vb = fit(1000, 300, { w: 800, h: 400 });
  assert.ok(Math.abs(vb.w / vb.h - 2) < 1e-9);
  assert.ok(vb.w >= 1000 && vb.h >= 300);
});

test('a four-layer graph keeps every card inside the fitted width', () => {
  const layers = ['data_center', 'site', 'node', 'sandbox'];
  const nodes = [
    node('dc1', 'data_center', 'Berlin'), node('dc2', 'data_center', 'default'),
    node('s1', 'site', 'Main site'), node('s2', 'site', 'No site'),
    node('n1', 'node', 'local'),
    node('b1', 'sandbox', 'claude-01'), node('b2', 'sandbox', 'claude-own'), node('b3', 'sandbox', 'desktop-01'),
  ];
  const edges = [
    { from: 'dc1', to: 's1' }, { from: 'dc2', to: 's2' },
    { from: 's1', to: 'n1' }, { from: 's2', to: 'n1' },
    { from: 'n1', to: 'b1' }, { from: 'n1', to: 'b2' }, { from: 'n1', to: 'b3' },
  ];
  const out = layout(nodes, edges, layers);
  const right = Math.max(...[...out.positions.values()].map((p) => p.x + DEFAULTS.card.w));
  assert.ok(out.width >= right, `width ${out.width} must hold the right edge ${right}`);
  for (const [id, p] of out.positions) assert.ok(p.x + DEFAULTS.card.w <= out.width, `${id} sticks out`);
  // With the home page's viewport the fitted viewBox still shows the whole last column.
  const vb = fit(out.width, out.height, { w: 720, h: 420 });
  assert.ok(vb.x + vb.w >= right);
  assert.ok(vb.x <= 0);
});

test('the column titles have their own band above the first card', () => {
  const out = layout([node('a', 'x'), node('b', 'y')], [{ from: 'a', to: 'b' }], ['x', 'y']);
  assert.ok(out.titleY < out.top, 'the title baseline sits above the first card');
  assert.ok(out.top >= out.pad + out.band, 'the cards start below the whole title band');
  assert.ok(out.positions.get('a').y >= out.top);
  assert.ok(out.height >= out.top + DEFAULTS.card.h + out.pad);
});

test('zoom keeps the point under the cursor', () => {
  const vb = { x: 0, y: 0, w: 100, h: 100 };
  const z = zoomAt(vb, 0.5, 0.25, 0.25);
  assert.equal(z.x + z.w * 0.25, 25);
});

test('thickness grows with requests and stops at 6', () => {
  assert.equal(thickness(0), 1);
  assert.ok(thickness(7) > thickness(1));
  assert.equal(thickness(1e12), 6);
});

test('an edge path starts at the right side of the first card', () => {
  assert.ok(edgePath({ x: 0, y: 0 }, { x: 220, y: 64 }).startsWith('M 176 26'));
});

test('status words fold into tones', () => {
  assert.equal(toneOf('running'), 'ok');
  assert.equal(toneOf('degraded'), 'warn');
  assert.equal(toneOf('failed'), 'bad');
  assert.equal(toneOf('whatever'), 'muted');
});

test('the cluster answer gains an egress card and a refused edge', () => {
  const out = fromCluster(
    [{ id: 'g', kind: 'gateway', name: 'gw', status: 'healthy' }, { id: 's', kind: 'sandbox', name: 'box', status: 'running' },
      { id: 'd', kind: 'destination', name: 'x.org', port: 443 }],
    [{ from: 'g', to: 's', kind: 'hosts' }, { from: 's', to: 'd', kind: 'egress', requests: 3, denied: 2 }],
  );
  assert.ok(out.nodes.some((n) => n.id === 'egress:g'));
  assert.ok(out.edges.some((e) => e.to === 'egress:g'));
  assert.equal(out.edges.find((e) => e.to === 'd').refused, true);
  assert.equal(out.nodes.find((n) => n.id === 'd').label, 'x.org:443');
});

test('the stack answer maps layer indexes to names', () => {
  const out = fromStack({
    layers: [{ index: 0, label: 'You' }, { index: 1, label: 'Sandbox' }],
    nodes: [{ id: 'a', kind: 'user', name: 'me', status: 'ok', layer: 0, state: 'x' }, { id: 'b', kind: 'sandbox', name: 'box', status: 'error', layer: 1 }],
    edges: [{ from: 'a', to: 'b', status: 'error' }],
  });
  assert.deepEqual(out.layers, ['You', 'Sandbox']);
  assert.equal(out.nodes[1].status, 'bad');
  assert.equal(out.edges[0].refused, true);
});
