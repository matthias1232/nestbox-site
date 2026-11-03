// Pure helpers for the inventory tree: filtering, paths and breadcrumbs (contracts C1, C3).
import { paths } from '../../routing.js';

// Route-kind names (C1) and tree-kind names (C3) that mean the same node.
const KIND_ALIASES = {
  dc: 'data_center',
  admin: 'sandbox',
  box: 'sandbox',
  terminal: 'sandbox',
  desktop: 'sandbox',
};

// The console address of a tree node, by kind (contract C1).
export function pathFor(node) {
  if (!node) return paths.home();
  switch (node.kind) {
    case 'organization': return paths.home();
    case 'data_center': return paths.dc(node.id);
    case 'site': return paths.site(node.id);
    case 'node': return paths.node(node.id);
    case 'sandbox': return paths.sandbox(node.id);
    case 'user': return `/sandboxes?owner=${encodeURIComponent(node.id)}`;
    case 'model': return paths.model(node.id);
    case 'install': return node.node_id ? paths.node(node.node_id) : paths.home();
    case 'cluster': return paths.cluster(node.id);
    default: return paths.home();
  }
}

// Keep nodes whose name matches `text` (case-insensitive) or that have a matching descendant.
// A matching node keeps all its children so the user sees what is inside it.
export function filterTree(root, text) {
  const needle = String(text || '').trim().toLowerCase();
  if (!root || !needle) return root;
  function walk(node) {
    const own = String(node.name || '').toLowerCase().includes(needle);
    if (own) return node;
    const children = (node.children || []).map(walk).filter(Boolean);
    return children.length ? { ...node, children } : null;
  }
  return walk(root) || { ...root, children: [] };
}

// Every node id that has children, so a filter can show all matches expanded.
export function expandableIds(root) {
  const ids = new Set();
  (function walk(node) {
    if (!node) return;
    if ((node.children || []).length) ids.add(node.id);
    (node.children || []).forEach(walk);
  })(root);
  return ids;
}

// The nodes a fresh sidebar shows open (V18): the organisation plus every data center, so the
// structure below it is visible without a click. Children of a data center stay closed; the
// user's own expansion wins from the first toggle on.
export function defaultExpanded(root) {
  const ids = new Set();
  if (!root) return ids;
  ids.add(root.id);
  (function walk(node) {
    (node.children || []).forEach((child) => {
      if (child.kind === 'data_center') ids.add(child.id);
      walk(child);
    });
  })(root);
  return ids;
}

// The nodes from the root down to (kind, id), or [] when it is not in the tree.
function chain(root, kind, id) {
  const want = KIND_ALIASES[kind] || kind;
  function walk(node, trail) {
    const here = [...trail, node];
    if (node.kind === want && String(node.id) === String(id)) return here;
    for (const child of node.children || []) {
      const found = walk(child, here);
      if (found) return found;
    }
    return null;
  }
  return root ? walk(root, []) || [] : [];
}

// Breadcrumb items [{ label, path }] from the root to the node (kind, id); [] when unknown.
export function treePathFor(root, kind, id) {
  return chain(root, kind, id).map((node) => ({ label: node.name, path: pathFor(node) }));
}

// The ids of the ancestors of (kind, id), so the sidebar can open the way to the current place.
export function ancestorIds(root, kind, id) {
  return chain(root, kind, id).slice(0, -1).map((node) => node.id);
}
