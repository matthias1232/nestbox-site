// Pure logic of the mount editor (contract C10): rows, validation, the PUT body, policy path lists.

// Mount rows from the GET answer's `roots`.
export function toRows(effective) {
  return ((effective && effective.roots) || []).map((root) => ({
    root_id: root.root_id,
    slug: root.slug,
    kind: root.kind,
    owner: root.owner,
    node: root.node,
    mount_path: root.mount_path,
    read_only: !!root.read_only,
    state: root.state || 'mounted',
    bytes_used: root.bytes_used ?? null,
  }));
}

// Flip one row between read-only and read-write; returns new rows.
export function flip(rows, rootId) {
  return rows.map((row) => (row.root_id === rootId ? { ...row, read_only: !row.read_only } : row));
}

// Change the mount path of one row; returns new rows.
export function setPath(rows, rootId, path) {
  return rows.map((row) => (row.root_id === rootId ? { ...row, mount_path: path } : row));
}

// The default mount path for a root: projects under /work, agent homes under .agent-state.
export function suggestPath(root) {
  const slug = String((root && root.slug) || 'data').replace(/[^a-zA-Z0-9._-]/g, '-');
  if (root && root.kind === 'agent_home') return `/home/nestbox/.agent-state/${slug}`;
  return `/work/${slug}`;
}

// Add an available root as a read-write row with a suggested path; no-op when already present.
export function add(rows, root) {
  if (rows.some((row) => row.root_id === root.root_id)) return rows;
  return [
    ...rows,
    {
      root_id: root.root_id,
      slug: root.slug,
      kind: root.kind,
      owner: root.owner,
      node: root.node,
      mount_path: suggestPath(root),
      read_only: false,
      state: 'pending_recreate',
      bytes_used: root.bytes_used ?? null,
    },
  ];
}

// Remove one row; returns new rows.
export function remove(rows, rootId) {
  return rows.filter((row) => row.root_id !== rootId);
}

// Why a path is not acceptable, or null. Same rules as the server: absolute, no `..`, unique.
export function validatePath(path, others = []) {
  const value = String(path || '').trim();
  if (!value.startsWith('/')) return 'The path must be absolute (start with /).';
  if (value.split('/').includes('..')) return 'The path must not contain "..".';
  if (value === '/') return 'The path must not be the root.';
  if (others.includes(value)) return 'Another mount already uses this path.';
  return null;
}

// Validation message per root id for all rows (only rows with a problem appear).
export function errors(rows) {
  const out = {};
  rows.forEach((row) => {
    const others = rows.filter((other) => other.root_id !== row.root_id).map((other) => other.mount_path.trim());
    const message = validatePath(row.mount_path, others);
    if (message) out[row.root_id] = message;
  });
  return out;
}

// The PUT body for the rows; `apply` is 'now' or 'later'.
export function bodyFor(rows, apply) {
  return {
    roots: rows.map((row) => ({ root_id: row.root_id, mount_path: row.mount_path.trim(), read_only: !!row.read_only })),
    apply,
  };
}

// True when the rows differ from the original rows (set, path or mode).
export function changed(rows, original) {
  if (rows.length !== original.length) return true;
  return rows.some((row) => {
    const before = original.find((other) => other.root_id === row.root_id);
    return !before || before.mount_path !== row.mount_path || before.read_only !== row.read_only;
  });
}

// The read-only and read-write path lists of a policy spec.
export function policyLists(spec) {
  const fs = (spec && spec.filesystem_policy) || {};
  return { ro: [...(fs.read_only || [])], rw: [...(fs.read_write || [])] };
}

// A copy of the spec with the filesystem path lists replaced.
export function setPolicyLists(spec, ro, rw) {
  const base = spec || {};
  return { ...base, filesystem_policy: { ...(base.filesystem_policy || {}), read_only: [...ro], read_write: [...rw] } };
}
