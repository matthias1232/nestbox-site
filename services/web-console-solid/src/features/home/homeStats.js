// Pure helpers for the home page: totals, health rows and the topology graph (contract C3).
// No JSX, no Solid: tested with `node --test homeStats.test.js`.

// Visit every node of the tree, parents first.
function walk(node, visit) {
  if (!node) return;
  visit(node);
  for (const child of node.children || []) walk(child, visit);
}

// Count what the installation holds; `alerts` is the sum of the sandboxes' own alert counts.
export function totals(root) {
  const out = { dataCenters: 0, sites: 0, nodes: 0, nodesOnline: 0, sandboxes: 0, running: 0, allowAll: 0, alerts: 0, gpuNodes: 0 };
  walk(root, (node) => {
    if (node.kind === 'data_center') out.dataCenters += 1;
    else if (node.kind === 'site') out.sites += 1;
    else if (node.kind === 'node') {
      out.nodes += 1;
      if (node.status === 'online') out.nodesOnline += 1;
      if (node.gpu) out.gpuNodes += 1;
    } else if (node.kind === 'sandbox') {
      out.sandboxes += 1;
      if (node.status === 'running') out.running += 1;
      if (node.allow_all) out.allowAll += 1;
      out.alerts += Number(node.alerts) || 0;
    }
  });
  return out;
}

// What needs attention, most serious first: offline nodes, failed sandboxes, drifted tunnels,
// allow-all sandboxes.
export function health(root) {
  const offline = [];
  const failed = [];
  const drifted = [];
  const open = [];
  walk(root, (node) => {
    const row = { kind: node.kind, id: node.id, name: node.name, status: node.status };
    if (node.kind === 'node') {
      if (node.status === 'offline') offline.push({ ...row, why: 'Node is offline' });
      else if (node.tunnel_state && node.tunnel_state !== 'up') drifted.push({ ...row, why: `Tunnel is ${node.tunnel_state}` });
    } else if (node.kind === 'sandbox') {
      if (node.status === 'failed') failed.push({ ...row, why: 'Sandbox failed' });
      if (node.allow_all) open.push({ ...row, why: 'Allow-all is on: nothing is blocked' });
    }
  });
  return [...offline, ...failed, ...drifted, ...open];
}

// The layers of the home map, left to right.
export const LAYERS = ['data_center', 'site', 'node', 'sandbox'];

// Turn the tree into TopologyCanvas nodes and edges (parent to child) for the four layers.
export function graphFromTree(root) {
  const nodes = [];
  const edges = [];
  const visit = (node, parent) => {
    const keep = LAYERS.includes(node.kind);
    let next = parent;
    if (keep) {
      nodes.push({
        id: node.id,
        kind: node.kind,
        label: node.name,
        status: node.status,
        layer: node.kind,
        meta: { running: node.counts?.running, alerts: node.counts?.alerts ?? node.alerts },
      });
      if (parent) edges.push({ from: parent, to: node.id });
      next = node.id;
    }
    for (const child of node.children || []) visit(child, next);
  };
  visit(root, null);
  return { nodes, edges };
}

// The console path a map card or health row opens; null for kinds without a page.
export function pathFor(kind, id) {
  if (kind === 'data_center') return `/dc/${id}`;
  if (kind === 'site') return `/site/${id}`;
  if (kind === 'node') return `/node/${id}`;
  if (kind === 'sandbox') return `/sandboxes/${id}`;
  return null;
}
