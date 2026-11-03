// Pure layout for TopologyCanvas: where each card goes, how an edge curves, how to fit the view.
// No JSX, no Solid: tested with `node --test topologyLayout.test.js`.

// The defaults of contract F16: column pitch, row pitch and card size, in SVG units.
export const DEFAULTS = { colGap: 220, rowGap: 64, card: { w: 176, h: 52 } };

// Space around the drawing, and the height of the layer titles above the first row.
const PAD = 24;
const HEADER = 28;

// Put every node in the column of its layer and order the rows.
//
// The first column is sorted by label. Every later column is placed by a barycenter pass:
// a node sits at the mean row of the nodes that point to it from earlier columns, and nodes
// without such a parent go below the others, sorted by label. Rows never overlap.
export function layout(nodes, edges, layers, options = {}) {
  const colGap = options.colGap ?? DEFAULTS.colGap;
  const rowGap = options.rowGap ?? DEFAULTS.rowGap;
  const card = options.card ?? DEFAULTS.card;
  const positions = new Map();
  const column = new Map(layers.map((name, index) => [name, index]));
  const byColumn = layers.map(() => []);
  for (const node of nodes || []) {
    if (column.has(node.layer)) byColumn[column.get(node.layer)].push(node);
  }
  const columnOf = new Map();
  byColumn.forEach((list, index) => list.forEach((node) => columnOf.set(node.id, index)));
  const parents = new Map();
  for (const edge of edges || []) {
    const from = columnOf.get(edge.from);
    const to = columnOf.get(edge.to);
    if (from === undefined || to === undefined || from >= to) continue;
    if (!parents.has(edge.to)) parents.set(edge.to, []);
    parents.get(edge.to).push(edge.from);
  }
  const byLabel = (a, b) => String(a.label ?? a.id).localeCompare(String(b.label ?? b.id));
  const top = PAD + HEADER;
  let rows = 0;
  byColumn.forEach((list, index) => {
    const wanted = new Map();
    for (const node of list) {
      const ys = (parents.get(node.id) || []).map((id) => positions.get(id)?.y).filter((y) => y !== undefined);
      wanted.set(node.id, ys.length ? ys.reduce((sum, y) => sum + y, 0) / ys.length : null);
    }
    list.sort((a, b) => {
      const ya = wanted.get(a.id);
      const yb = wanted.get(b.id);
      if (ya === null && yb === null) return byLabel(a, b);
      if (ya === null) return 1;
      if (yb === null) return -1;
      return ya - yb || byLabel(a, b);
    });
    const step = card.h + (rowGap - card.h > 8 ? rowGap - card.h : 12);
    let previous = null;
    for (const node of list) {
      const wish = wanted.get(node.id);
      let y = wish === null ? (previous === null ? top : previous + step) : Math.max(wish, previous === null ? top : previous + step);
      if (y < top) y = top;
      positions.set(node.id, { x: PAD + index * colGap, y });
      previous = y;
    }
    rows = Math.max(rows, previous === null ? 0 : previous + card.h);
  });
  const width = PAD * 2 + Math.max(0, layers.length - 1) * colGap + card.w;
  const height = rows + PAD;
  return { positions, width, height: Math.max(height, top + card.h + PAD), top, pad: PAD };
}

// A cubic bezier from the right middle of card `a` to the left middle of card `b`
// (the other way round when `b` lies to the left of `a`).
export function edgePath(a, b, card = DEFAULTS.card) {
  const back = b.x < a.x;
  const x1 = back ? a.x : a.x + card.w;
  const x2 = back ? b.x + card.w : b.x;
  const y1 = a.y + card.h / 2;
  const y2 = b.y + card.h / 2;
  const mid = (x1 + x2) / 2;
  return `M ${x1} ${y1} C ${mid} ${y1}, ${mid} ${y2}, ${x2} ${y2}`;
}

// Stroke width of an edge from its request count: 1 for none, at most 6.
export function thickness(requests) {
  return Math.min(6, 1 + Math.log2(1 + Math.max(0, Number(requests) || 0)));
}

// The viewBox that shows the whole drawing centred, with the viewport's aspect ratio.
export function fit(width, height, viewport) {
  const aspect = viewport.w > 0 && viewport.h > 0 ? viewport.w / viewport.h : 1;
  let w = Math.max(width, 1);
  let h = w / aspect;
  if (h < height) {
    h = Math.max(height, 1);
    w = h * aspect;
  }
  return { x: (width - w) / 2, y: (height - h) / 2, w, h };
}

// Zoom a viewBox by `factor` (above 1 shows more) around the point (px, py) given as
// fractions 0..1 of the viewport, so the point under the cursor stays under the cursor.
export function zoomAt(vb, factor, px = 0.5, py = 0.5) {
  const w = vb.w * factor;
  const h = vb.h * factor;
  return { x: vb.x + (vb.w - w) * px, y: vb.y + (vb.h - h) * py, w, h };
}

// The status words every host may use, folded into the four tones of the canvas.
export function toneOf(status) {
  const value = String(status ?? '').toLowerCase();
  if (['ok', 'running', 'healthy', 'online', 'up', 'reachable', 'installed', 'allowed'].includes(value)) return 'ok';
  if (['warn', 'degraded', 'starting', 'provisioning', 'pending', 'attention'].includes(value)) return 'warn';
  if (['bad', 'error', 'failed', 'unhealthy', 'offline', 'blocked', 'refused', 'down', 'unreachable'].includes(value)) return 'bad';
  return 'muted';
}

// Map the cluster answer (supervisors, gateways, sandboxes, destinations; edges `hosts` and
// `egress`) to the nodes and edges of contract C15. One derived "policy → internet" card per
// gateway is added, as the old graph did, so the path sandbox → gateway egress stays visible.
export function fromCluster(nodes, edges) {
  const out = [];
  const gateways = [];
  for (const node of nodes || []) {
    const label = node.kind === 'destination' && node.port ? `${node.name}:${node.port}` : node.name;
    const meta = {};
    if (node.node_name) meta.node = node.node_name;
    if (node.port) meta.port = node.port;
    if (node.protocol) meta.protocol = node.protocol;
    if (node.drifted) meta.drift = 'config drift';
    if (node.kind === 'gateway') gateways.push(node);
    out.push({
      id: node.id,
      kind: node.kind,
      label,
      status: node.drifted && toneOf(node.status) === 'ok' ? 'warn' : node.status,
      layer: node.kind,
      meta,
      source: node,
    });
  }
  for (const gateway of gateways) {
    out.push({
      id: `egress:${gateway.id}`,
      kind: 'egress',
      label: 'policy → internet',
      status: gateway.drifted ? 'degraded' : gateway.status,
      layer: 'egress',
      meta: {},
    });
  }
  const gatewayOf = new Map();
  const drawn = [];
  for (const edge of edges || []) {
    if (edge.kind === 'hosts') gatewayOf.set(edge.to, edge.from);
    const refused = Boolean(edge.blocked) || (edge.denied || 0) > 0;
    drawn.push({
      from: edge.from,
      to: edge.to,
      requests: edge.requests || 0,
      refused,
      label: edge.kind === 'egress' ? `${edge.requests || 0} req · ${edge.denied || 0} blocked` : undefined,
    });
  }
  for (const node of nodes || []) {
    const gateway = gatewayOf.get(node.id);
    if (node.kind === 'sandbox' && gateway) drawn.push({ from: node.id, to: `egress:${gateway}`, requests: 0, refused: false });
  }
  return { nodes: out, edges: drawn };
}

// Map the stack-topology answer (layers `{index,label}`, nodes with `layer` index, edges with
// a `status`) to C15. Layer names are the server's labels; an `error` edge is drawn refused.
export function fromStack(topology) {
  const layers = [...(topology?.layers || [])].sort((a, b) => a.index - b.index);
  const nameOf = new Map(layers.map((layer) => [layer.index, layer.label]));
  const used = new Set((topology?.nodes || []).map((node) => node.layer));
  const names = layers.filter((layer) => used.has(layer.index)).map((layer) => layer.label);
  const nodes = (topology?.nodes || []).map((node) => ({
    id: node.id,
    kind: node.kind,
    label: node.name,
    status: node.status === 'error' ? 'bad' : node.status === 'idle' || node.status === 'absent' ? 'muted' : node.status,
    layer: nameOf.get(node.layer),
    meta: { state: node.state },
    source: node,
  }));
  const edges = (topology?.edges || []).map((edge) => ({
    from: edge.from,
    to: edge.to,
    requests: edge.requests || 0,
    refused: edge.status === 'error' || (edge.denied || 0) > 0,
    label: edge.label,
  }));
  return { nodes, edges, layers: names };
}
