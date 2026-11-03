// Tab lists per page kind and role, plus the small pure helpers the inventory pages share.

const RANK = { viewer: 1, member: 2, operator: 3, admin: 4 };

const NODE_TABS = [
  { id: 'summary', label: 'Summary' },
  { id: 'sandboxes', label: 'Sandboxes' },
  { id: 'models', label: 'Models' },
  { id: 'images', label: 'Images', min: 'operator' },
  { id: 'traffic', label: 'Traffic', min: 'operator' },
  { id: 'logs', label: 'Logs', min: 'operator' },
];

const SITE_TABS = [
  { id: 'summary', label: 'Summary' },
  { id: 'nodes', label: 'Nodes', min: 'operator' },
  { id: 'sandboxes', label: 'Sandboxes' },
];

const DC_TABS = [
  { id: 'summary', label: 'Summary' },
  { id: 'sites', label: 'Sites & nodes', min: 'operator' },
  { id: 'sandboxes', label: 'Sandboxes' },
  { id: 'models', label: 'Models', min: 'operator' },
  { id: 'policies', label: 'Policies', min: 'operator' },
  { id: 'permissions', label: 'Permissions', min: 'admin' },
  { id: 'usage', label: 'Usage', min: 'operator' },
];

const BY_KIND = { node: NODE_TABS, site: SITE_TABS, dc: DC_TABS };

// Whether `role` reaches `min`; an unknown role is a viewer.
function allowed(role, min) {
  return (RANK[role] || 1) >= (RANK[min] || 1);
}

// The tabs of a page kind ('dc' | 'site' | 'node') for a role. A member sees Summary and Sandboxes only.
export function tabsFor(kind, role) {
  const list = BY_KIND[kind] || [];
  if (role === 'member' || role === 'viewer') {
    return list.filter((tab) => tab.id === 'summary' || tab.id === 'sandboxes').map(({ id, label }) => ({ id, label }));
  }
  return list.filter((tab) => allowed(role, tab.min)).map(({ id, label }) => ({ id, label }));
}

// The tab to show: the requested one when the role may see it, else the first (Summary).
export function activeTab(kind, role, requested) {
  const tabs = tabsFor(kind, role);
  return tabs.some((tab) => tab.id === requested) ? requested : tabs[0]?.id || 'summary';
}

// The `?tab=` query value of the current address (empty string when absent).
export function tabFromSearch(search) {
  const match = /[?&]tab=([^&]*)/.exec(search || '');
  return match ? decodeURIComponent(match[1]) : '';
}

// An address with the tab in the query; Summary is the bare address.
export function tabPath(base, tab) {
  return !tab || tab === 'summary' ? base : `${base}?tab=${encodeURIComponent(tab)}`;
}

// Bytes as a binary-unit string ("12.5 GiB"); a missing value reads "–".
export function gib(bytes) {
  if (typeof bytes !== 'number' || !Number.isFinite(bytes) || bytes < 0) return '–';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${unit === 0 ? value : value.toFixed(1)} ${units[unit]}`;
}

// Bytes as a number of GiB with one decimal, for a gauge.
export function toGib(bytes) {
  return typeof bytes === 'number' && Number.isFinite(bytes) ? Math.round((bytes / 1073741824) * 10) / 10 : 0;
}

// The reading under a gauge arc: "41 / 64 GiB"; a machine that reports no total (VRAM today)
// reads "–" instead of a fake zero (V09).
export function pairText(usedBytes, totalBytes) {
  if (typeof totalBytes !== 'number' || !Number.isFinite(totalBytes) || totalBytes <= 0) return '–';
  return `${toGib(usedBytes)} / ${toGib(totalBytes)} GiB`;
}

// "3 min ago" style age of an ISO time; "–" when unknown.
export function ago(iso, now = Date.now()) {
  const then = Date.parse(iso || '');
  if (Number.isNaN(then)) return '–';
  const seconds = Math.max(0, Math.round((now - then) / 1000));
  if (seconds < 60) return `${seconds} s ago`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ago`;
  return `${Math.floor(seconds / 86400)} d ago`;
}

// An ISO instant as the local "YYYY-MM-DD HH:mm" an operator reads in a detail view (V16).
// A raw "2026-10-27T01:00:25.368741Z" says nothing about the operator's clock; unknown reads "–".
export function stamp(iso) {
  const ms = Date.parse(iso || '');
  if (Number.isNaN(ms)) return '–';
  const at = new Date(ms);
  const pad = (n) => String(n).padStart(2, '0');
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${pad(at.getHours())}:${pad(at.getMinutes())}`;
}

// The sandboxes, nodes and online nodes found under a tree node (any depth).
export function countBelow(node) {
  const out = { sandboxes: 0, running: 0, nodes: 0, online: 0, sites: 0 };
  (function walk(n) {
    if (n.kind === 'sandbox') {
      out.sandboxes += 1;
      if (n.status === 'running') out.running += 1;
    } else if (n.kind === 'node') {
      out.nodes += 1;
      if (n.status === 'online') out.online += 1;
    } else if (n.kind === 'site') {
      out.sites += 1;
    }
    (n.children || []).forEach(walk);
  })(node || {});
  return out;
}

// Find a node of the tree by kind and id (kind 'dc' means 'data_center'); null when absent.
export function findNode(root, kind, id) {
  const want = kind === 'dc' ? 'data_center' : kind;
  let found = null;
  (function walk(n) {
    if (found || !n) return;
    if (n.kind === want && String(n.id) === String(id)) found = n;
    else (n.children || []).forEach(walk);
  })(root);
  return found;
}

// Every node (kind 'node') below a tree node, flat.
export function nodesBelow(node) {
  const out = [];
  (function walk(n) {
    if (n.kind === 'node') out.push(n);
    (n.children || []).forEach(walk);
  })(node || {});
  return out;
}

// Worst status word of a list for a header badge: bad > warn > ok.
export function worstStatus(statuses) {
  if (statuses.some((s) => s === 'bad' || s === 'offline')) return 'bad';
  if (statuses.some((s) => s === 'warn' || s === 'degraded' || s === 'unknown')) return 'warn';
  return 'ok';
}
