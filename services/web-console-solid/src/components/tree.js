// Pure helpers behind Tree: flatten the visible rows and move the cursor.

// The rows currently visible, depth first: a node's children appear only when its id is in `expanded`.
export function visibleRows(root, expanded) {
  const rows = [];
  const walk = (node, depth) => {
    const children = node.children || [];
    rows.push({ node, depth, hasChildren: children.length > 0 });
    if (children.length > 0 && expanded && expanded.has(node.id)) {
      for (const child of children) walk(child, depth + 1);
    }
  };
  if (root) walk(root, 0);
  return rows;
}

// Keep the nodes whose name matches `filter` and every ancestor of a match, so the path stays readable.
// Matching is case-insensitive; returns null when nothing matches.
export function filterTree(root, filter) {
  const needle = String(filter || '').trim().toLowerCase();
  if (!needle || !root) return root;
  const keep = (node) => {
    const children = (node.children || []).map(keep).filter(Boolean);
    const self = String(node.name || '').toLowerCase().includes(needle);
    if (!self && children.length === 0) return null;
    return { ...node, children };
  };
  return keep(root);
}

// The ids of every node that has children (a filter opens them all).
export function idsWithChildren(root) {
  const ids = [];
  const walk = (node) => {
    if ((node.children || []).length) ids.push(node.id);
    (node.children || []).forEach(walk);
  };
  if (root) walk(root);
  return ids;
}

// The index of the row the cursor moves to for a key: ArrowUp/ArrowDown step, Home/End jump.
// ArrowLeft on a collapsed or leaf row moves to its parent; ArrowRight on an open row to its first child.
export function nextIndex(rows, index, key, expanded) {
  if (rows.length === 0) return -1;
  const last = rows.length - 1;
  if (key === 'ArrowDown') return Math.min(last, index + 1);
  if (key === 'ArrowUp') return Math.max(0, index - 1);
  if (key === 'Home') return 0;
  if (key === 'End') return last;
  const row = rows[index];
  if (!row) return 0;
  if (key === 'ArrowRight' && row.hasChildren && expanded && expanded.has(row.node.id)) {
    return Math.min(last, index + 1);
  }
  if (key === 'ArrowLeft' && !(row.hasChildren && expanded && expanded.has(row.node.id))) {
    for (let i = index - 1; i >= 0; i -= 1) if (rows[i].depth < row.depth) return i;
  }
  return index;
}

// What ArrowLeft/ArrowRight should do to the expanded set: 'expand', 'collapse' or null.
export function toggleAction(rows, index, key, expanded) {
  const row = rows[index];
  if (!row || !row.hasChildren) return null;
  const open = expanded && expanded.has(row.node.id);
  if (key === 'ArrowRight' && !open) return 'expand';
  if (key === 'ArrowLeft' && open) return 'collapse';
  return null;
}
