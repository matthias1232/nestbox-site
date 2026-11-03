// The pure part of "which data center opens": the remembered choice of one user in this
// browser, and the notice for a data center that has no nodes.
//
// No DOM, no Solid, no storage call: the caller reads and writes localStorage, this file
// only decides. The stored value is a data-center id, never a token and never a secret.

// The localStorage key prefix; the user id follows, so two people on one browser keep
// their own choice.
export const STORAGE_PREFIX = 'nestbox:environment:';

// The storage key of one user, or null when there is no user to key on yet.
export function storageKey(userId) {
  return userId ? `${STORAGE_PREFIX}${userId}` : null;
}

// Which data center the console opens for one signed-in user.
//
// The remembered choice wins, but only while the server still lists that data center: a
// permission taken away must not leave the console on a data center it cannot read. Next
// comes the server's current one, and null when neither is reachable (the caller keeps
// what it already has).
export function pickEnvironment(environments, remembered, current) {
  const list = Array.isArray(environments) ? environments : [];
  const known = (id) => Boolean(id) && list.some((entry) => entry && entry.id === id);
  if (known(remembered)) return remembered;
  if (known(current)) return current;
  // `/environments` marks the server's current one; a caller that has only the list still
  // lands on it rather than on nothing.
  const flagged = list.find((entry) => entry && entry.current === true);
  return flagged ? flagged.id : null;
}

// The data centers of `GET /api/v1/organizations`, flattened, with their node counts.
//
// `GET /api/v1/environments` names the data centers a user may open but does not say what is
// inside them; the organization list does, as `environments[].nodes`. The console needs that
// count to answer one question on a first sign-in: is the data center the session opens empty
// while another one holds every node (U02)? An entry without an id is dropped, because a data
// center the console cannot name is one it cannot open.
export function environmentsWithNodes(organizations) {
  const out = [];
  (Array.isArray(organizations) ? organizations : []).forEach((organization) => {
    const entries = Array.isArray(organization?.environments) ? organization.environments : [];
    entries.forEach((entry) => {
      if (!entry || typeof entry.id !== 'string' || !entry.id) return;
      const nodes = Number(entry.nodes);
      out.push({ id: entry.id, name: entry.name || entry.id, nodes: Number.isFinite(nodes) ? nodes : 0 });
    });
  });
  return out;
}

// The data center a first sign-in opens: the current one while it has nodes, otherwise the
// first one that has any, otherwise null (nothing to switch to).
//
// The order is the promise this makes: the server's current data center is where the user
// already is, so it keeps the console where they expect it. Only an empty one gives way, and
// the console says so once instead of switching silently (U02).
export function firstUsable(environments, current) {
  const list = (Array.isArray(environments) ? environments : [])
    .filter((entry) => entry && typeof entry.id === 'string' && entry.id);
  const nodesOf = (id) => {
    const entry = list.find((candidate) => candidate.id === id);
    return entry ? Number(entry.nodes) || 0 : 0;
  };
  if (current && nodesOf(current) > 0) return current;
  const usable = list.find((entry) => (Number(entry.nodes) || 0) > 0);
  return usable ? usable.id : null;
}

// The data centers of an inventory tree (contract C3), root first, depth first.
function dataCenters(root) {
  const found = [];
  (function walk(node) {
    if (!node) return;
    if (node.kind === 'data_center') found.push(node);
    (node.children || []).forEach(walk);
  })(root);
  return found;
}

// The notice for an empty data center, or null when there is nothing to say.
//
// The console must not switch by itself: a silent switch hides that a data center is empty
// and leaves the user wondering which one they are looking at. So the caller gets the name
// of the current data center, the one with nodes, and its id for a button.
export function emptyDataCenterNotice(root, activeId) {
  if (!root || !activeId) return null;
  const centers = dataCenters(root);
  const current = centers.find((node) => node.id === activeId);
  if (!current || (current.counts?.nodes ?? 0) > 0) return null;
  const fuller = centers.find((node) => node.id !== activeId && (node.counts?.nodes ?? 0) > 0);
  if (!fuller) return null;
  return {
    currentName: current.name || 'This data center',
    targetId: fuller.id,
    targetName: fuller.name || 'the other data center',
  };
}
