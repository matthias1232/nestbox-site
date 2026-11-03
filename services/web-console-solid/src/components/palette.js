// Pure logic of the command palette: matching, grouping, filtering and keyboard movement.
//
// Nothing here touches the DOM, the API or Solid, so `palette.test.js` can prove it with
// plain `node --test`. A result item is `{ id, kind, name, subtitle, status, environment_id,
// path }`; `kind` is a server kind (contract C4) or `doc` / `menu`.

/** The nine kinds the server can search (contract C4), in the order the Kind chip lists them. */
export const KINDS = ['sandbox', 'node', 'model', 'cluster', 'policy', 'user', 'site', 'data_center', 'job'];

/** The statuses the Status chip offers. */
export const STATUSES = ['running', 'stopped', 'online', 'offline', 'failed'];

/** Group order and headings. `site` and `data_center` share the "Places" group. */
export const GROUPS = [
  { id: 'recent', label: 'Recent' },
  { id: 'sandbox', label: 'Sandboxes' },
  { id: 'node', label: 'Nodes' },
  { id: 'model', label: 'Models' },
  { id: 'cluster', label: 'Clusters' },
  { id: 'policy', label: 'Policies' },
  { id: 'user', label: 'Users' },
  { id: 'place', label: 'Places' },
  { id: 'job', label: 'Jobs' },
  { id: 'doc', label: 'Docs' },
  { id: 'menu', label: 'Menu' },
];

/** The human word for a kind, used on filter chips and result rows. */
export const KIND_LABELS = {
  sandbox: 'Sandbox',
  node: 'Node',
  model: 'Model',
  cluster: 'Cluster',
  policy: 'Policy',
  user: 'User',
  site: 'Site',
  data_center: 'Data center',
  job: 'Job',
  doc: 'Doc',
  menu: 'Menu',
};

function groupOf(kind) {
  if (kind === 'site' || kind === 'data_center') return 'place';
  return kind;
}

function norm(text) {
  return String(text ?? '').trim().toLowerCase();
}

/** Flatten `menuFor(role)` into searchable entries; routes that only run an action are skipped. */
export function menuEntries(menu, pathFor) {
  const out = [];
  for (const group of menu || []) {
    for (const route of group.routes || []) {
      if (route.action) continue;
      out.push({
        id: `menu:${route.id}`,
        kind: 'menu',
        name: route.label,
        subtitle: group.label,
        status: null,
        environment_id: null,
        path: pathFor(route.id),
      });
    }
  }
  return out;
}

/** Menu entries whose label contains `q`, best (prefix) matches first. Empty `q` matches nothing. */
export function matchMenu(entries, q) {
  const needle = norm(q);
  if (!needle) return [];
  const scored = [];
  for (const entry of entries || []) {
    const label = norm(entry.name);
    const at = label.indexOf(needle);
    if (at < 0) continue;
    scored.push({ entry, score: at === 0 ? 2 : 1 });
  }
  scored.sort((a, b) => b.score - a.score);
  return scored.map((row) => row.entry);
}

/**
 * Help documents matching `q`: title scores 3, a tag 2, a heading 1. A document found only
 * through a heading links to that heading. Empty `q` matches nothing.
 */
export function matchDocs(index, q, pathFor = (slug) => `/docs/${slug}`) {
  const needle = norm(q);
  if (!needle) return [];
  const scored = [];
  for (const doc of index || []) {
    const titleHit = norm(doc.title).includes(needle);
    const tagHit = (doc.tags || []).some((tag) => norm(tag).includes(needle));
    const heading = (doc.headings || []).find((entry) => norm(entry.text).includes(needle));
    const score = (titleHit ? 3 : 0) + (tagHit ? 2 : 0) + (heading ? 1 : 0);
    if (score === 0) continue;
    const anchor = !titleHit && heading ? `#${heading.id}` : '';
    scored.push({
      score,
      item: {
        id: `doc:${doc.slug}${anchor}`,
        kind: 'doc',
        name: doc.title,
        subtitle: anchor ? `${heading.text}` : doc.summary || '',
        status: null,
        environment_id: null,
        path: `${pathFor(doc.slug)}${anchor}`,
      },
    });
  }
  scored.sort((a, b) => b.score - a.score);
  return scored.map((row) => row.item);
}

/** Turn the server's `items` (contract C4) into result items with a stable `id`. */
export function serverItems(items) {
  return (items || []).map((item) => ({
    id: `${item.kind}:${item.id}`,
    kind: item.kind,
    name: item.name,
    subtitle: item.subtitle || '',
    status: item.status || null,
    environment_id: item.environment_id || null,
    path: item.path,
  }));
}

/**
 * Merge the four sources into display groups. With an empty query only Recent shows; with a
 * query Recent is left out and the groups follow the fixed order in `GROUPS`.
 */
export function mergeResults(server, menu, docs, recent, q) {
  const buckets = new Map(GROUPS.map((group) => [group.id, []]));
  if (!norm(q)) {
    buckets.get('recent').push(...(recent || []));
  } else {
    for (const item of server || []) {
      const bucket = buckets.get(groupOf(item.kind));
      if (bucket) bucket.push(item);
    }
    buckets.get('doc').push(...(docs || []));
    buckets.get('menu').push(...(menu || []));
  }
  return GROUPS.map((group) => ({ id: group.id, label: group.label, items: buckets.get(group.id) })).filter(
    (group) => group.items.length > 0,
  );
}

/**
 * Apply the three chips to the groups. `kind` keeps one kind (docs and menu entries drop out
 * unless that is the kind); `environment` and `status` keep only items that carry that value.
 */
export function applyFilters(groups, { kind = '', environment = '', status = '' } = {}) {
  return groups
    .map((group) => ({
      ...group,
      items: group.items.filter((item) => {
        if (kind && item.kind !== kind) return false;
        if (environment && item.environment_id !== environment) return false;
        if (status && item.status !== status) return false;
        return true;
      }),
    }))
    .filter((group) => group.items.length > 0);
}

/** Every item of the groups in display order, for arrow-key movement. */
export function flatten(groups) {
  return groups.flatMap((group) => group.items);
}

/** The next active index after pressing an arrow key; wraps at both ends, -1 when empty. */
export function moveIndex(current, delta, count) {
  if (count <= 0) return -1;
  return (((current + delta) % count) + count) % count;
}

/** The parameters for the server search; empty chips are omitted. */
export function searchParams(q, { kind = '', environment = '', status = '' } = {}) {
  const params = { q: String(q).trim(), limit: 20 };
  if (kind && KINDS.includes(kind)) params.kind = kind;
  if (environment) params.environment = environment;
  if (status) params.status = status;
  return params;
}

/** A counter that tells whether an answer still belongs to the newest request. */
export function createLatest() {
  let counter = 0;
  return {
    // Start a request and get its number.
    begin: () => (counter += 1),
    // True while no newer request started.
    isCurrent: (id) => id === counter,
  };
}

/** The small record kept in the persisted recents: ids and labels only, never details. */
export function recentRecord(item) {
  return { id: item.id, kind: item.kind, name: item.name, subtitle: '', status: null, environment_id: null, path: item.path };
}
