// Which data center a place lives in, from the inventory tree (contract C3).
//
// The session's data center scopes every read, so a link that names an entity of another data
// center answers "The requested resource does not exist" (V15, seen on `/box/<id>` while the
// session sat in Berlin). The tree the sidebar already holds is the only client-side map from
// an entity id to its data center, so this walks it by id.
//
// `place` is what `parsePath` returns (contract C1). Menu pages, `home`, `my` and `docs` name
// no entity, and an id the tree does not contain answers null: in both cases there is nothing
// to switch, and the page itself says what is missing.

// Route kind (C1) to tree kind (C3); the other kinds carry the same name in both.
const TREE_KIND = { dc: 'data_center', admin: 'sandbox', box: 'sandbox' };

// The kinds this helper answers for. `cluster` is not one of them: a cluster id is not walked
// here, so its page never moves the session by surprise.
const WALKABLE = new Set(['dc', 'site', 'node', 'admin', 'box', 'model']);

/**
 * The id of the data center that contains `place`, or null.
 *
 * The first data center on the path decides: the tree nests each entity under exactly one, so
 * a node or sandbox id answers the data center above it, and a data center answers itself.
 * Missing arguments, a kind without a tree entity and an unknown id all answer null.
 */
export function placeEnvironment(root, place) {
  if (!root || !place || !WALKABLE.has(place.kind) || !place.id) return null;
  const want = TREE_KIND[place.kind] || place.kind;
  const id = String(place.id);

  function walk(node, dataCenterId) {
    const own = node.kind === 'data_center' ? node.id : dataCenterId;
    if (node.kind === want && String(node.id) === id) return own;
    for (const child of node.children || []) {
      const found = walk(child, own);
      if (found) return found;
    }
    return null;
  }

  return walk(root, null);
}
