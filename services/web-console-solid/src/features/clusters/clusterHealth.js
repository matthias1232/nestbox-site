// Pure helpers for the model cluster pages: health wording, strategy labels, member rules.

// The three balancing strategies, each with the one sentence the form shows.
export const STRATEGIES = [
  { id: 'round_robin', label: 'Round robin', hint: 'Each request goes to the next member.' },
  { id: 'least_loaded', label: 'Least loaded', hint: 'Each request goes to the member with the fewest open requests.' },
  { id: 'failover', label: 'Failover', hint: 'Always the first healthy member.' },
];

// The human label of a strategy id; an unknown id is shown as it is.
export function strategyLabel(strategy) {
  const found = STRATEGIES.find((s) => s.id === strategy);
  return found ? found.label : String(strategy || '');
}

// How many members answer: `healthy_members` when the API sends it, else counted from the members.
function healthyCount(cluster) {
  if (Number.isInteger(cluster.healthy_members)) return cluster.healthy_members;
  return (cluster.members || []).filter((m) => m.state === 'installed' && m.provider_status === 'reachable').length;
}

// The tone and the words for a cluster: "3 of 4 members healthy".
export function health(cluster) {
  const total = (cluster.members || []).length;
  if (total === 0) return { tone: 'warn', text: 'No members' };
  const healthy = Math.min(total, healthyCount(cluster));
  const text = `${healthy} of ${total} member${total === 1 ? '' : 's'} healthy`;
  if (healthy === total) return { tone: 'ok', text };
  if (healthy === 0) return { tone: 'bad', text };
  return { tone: 'warn', text };
}

// A member may be removed unless it is the last one (a cluster without members serves nothing).
export function canRemove(cluster, installId) {
  const members = cluster.members || [];
  return members.length > 1 && members.some((m) => m.install_id === installId);
}

// The model keys that have at least one install, with their display names, sorted by name.
export function modelsWithInstalls(installs) {
  const seen = new Map();
  for (const row of installs || []) {
    if (!seen.has(row.model_key)) seen.set(row.model_key, row.model_name || row.model_key);
  }
  return [...seen].map(([key, name]) => ({ key, name })).sort((a, b) => a.name.localeCompare(b.name));
}

// The installs of `modelKey` that a cluster could still take: installed, and not in `memberIds`.
export function pickableInstalls(installs, modelKey, memberIds = []) {
  const taken = new Set(memberIds);
  return (installs || []).filter((row) => row.model_key === modelKey && row.state === 'installed' && !taken.has(row.id));
}

// The 409 answer of a delete names the sandboxes that use the cluster; pull the names out.
export function usersFromError(error) {
  const details = error && error.details;
  const list = details && (details.sandboxes || details.used_by);
  if (!Array.isArray(list)) return [];
  return list.map((entry) => (typeof entry === 'string' ? entry : entry.name || entry.id)).filter(Boolean);
}
