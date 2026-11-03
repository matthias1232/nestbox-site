// Pure filters of the live traffic page: kind, decision and target (sandbox or node).

// The kind chips, in display order. `proxy` is plain network traffic through the egress proxy.
export const KIND_CHIPS = [
  { id: 'all', label: 'All' },
  { id: 'proxy', label: 'Proxy' },
  { id: 'dns', label: 'DNS' },
  { id: 'mcp', label: 'MCP' },
  { id: 'inference', label: 'Inference' },
];

// The decision chips, in display order.
export const DECISION_CHIPS = [
  { id: 'all', label: 'All' },
  { id: 'allowed', label: 'Allowed' },
  { id: 'refused', label: 'Refused' },
  { id: 'would_deny', label: 'Would deny' },
];

// Which kind chip a row belongs to.
export function kindOf(row) {
  if (row.kind === 'dns' || row.channel === 'dns') return 'dns';
  if (row.kind === 'mcp' || row.channel === 'mcp') return 'mcp';
  if (row.kind === 'inference' || row.channel === 'inference') return 'inference';
  return 'proxy';
}

// Whether a row would have been refused but was only observed (audit mode or a flag).
export function isWouldDeny(row) {
  return row.decision === 'audit' || row.would_deny === true || row.wouldDeny === true;
}

// Which decision chip a row belongs to.
export function decisionOf(row) {
  if (row.decision === 'deny') return 'refused';
  if (isWouldDeny(row)) return 'would_deny';
  return 'allowed';
}

// Whether a row is a refused one (the red lane shows these).
export function isRefused(row) {
  return row.decision === 'deny';
}

// The node name of a row: its own field, else the sandbox-to-node map.
function nodeOf(row, nodeBySandbox) {
  return row.node_name || row.nodeName || (nodeBySandbox ? nodeBySandbox[row.sandboxId] : null) || null;
}

// Rows that pass the three filters. `target` is '' | 'sandbox:<id>' | 'node:<name>'.
export function applyFilters(rows, { kind = 'all', decision = 'all', target = '' } = {}, nodeBySandbox = null) {
  const [targetKind, ...rest] = String(target || '').split(':');
  const targetValue = rest.join(':');
  return rows.filter((row) => {
    if (kind !== 'all' && kindOf(row) !== kind) return false;
    if (decision !== 'all' && decisionOf(row) !== decision) return false;
    if (targetKind === 'sandbox' && row.sandboxId !== targetValue) return false;
    if (targetKind === 'node' && nodeOf(row, nodeBySandbox) !== targetValue) return false;
    return true;
  });
}

// Counts per kind chip and per decision chip, over the rows after the other filters
// are applied, so every chip says how many rows it would show.
export function counts(rows, { kind = 'all', decision = 'all', target = '' } = {}, nodeBySandbox = null) {
  const result = { kind: { all: 0 }, decision: { all: 0 } };
  for (const chip of KIND_CHIPS) result.kind[chip.id] = 0;
  for (const chip of DECISION_CHIPS) result.decision[chip.id] = 0;
  for (const row of applyFilters(rows, { kind: 'all', decision, target }, nodeBySandbox)) {
    result.kind.all += 1;
    result.kind[kindOf(row)] += 1;
  }
  for (const row of applyFilters(rows, { kind, decision: 'all', target }, nodeBySandbox)) {
    result.decision.all += 1;
    result.decision[decisionOf(row)] += 1;
  }
  return result;
}

// Options for the third filter from an inventory tree (contract C3): nodes, then sandboxes.
export function targetOptions(tree) {
  const nodes = [];
  const sandboxes = [];
  const walk = (item) => {
    if (!item) return;
    if (item.kind === 'node') nodes.push({ value: `node:${item.name}`, label: `Node ${item.name}` });
    if (item.kind === 'sandbox') sandboxes.push({ value: `sandbox:${item.id}`, label: item.name });
    (item.children || []).forEach(walk);
  };
  walk(tree);
  const byLabel = (a, b) => a.label.localeCompare(b.label);
  return [...nodes.sort(byLabel), ...sandboxes.sort(byLabel)];
}

// Sandbox id -> node name, from the same tree.
export function nodeBySandbox(tree) {
  const map = {};
  const walk = (item, node) => {
    if (!item) return;
    const here = item.kind === 'node' ? item.name : node;
    if (item.kind === 'sandbox' && here) map[item.id] = here;
    (item.children || []).forEach((child) => walk(child, here));
  };
  walk(tree, null);
  return map;
}

// Options for the third filter from a sandbox-name map (used when no tree is available).
export function sandboxOptions(names) {
  return Object.entries(names || {})
    .map(([id, name]) => ({ value: `sandbox:${id}`, label: name }))
    .sort((a, b) => a.label.localeCompare(b.label));
}
