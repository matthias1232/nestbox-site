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
