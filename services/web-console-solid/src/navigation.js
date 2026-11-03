// The console menu as data: which route belongs under which heading, and who sees it.
//
// A menu that lives inside the component is a menu nobody can test. Keeping it here
// means the grouping rules (every route in exactly one group, a hidden entry for a
// role that may not use it) are checked by a plain unit test, and a new route is one
// line in one place.
//
// The six groups answer six different questions an operator asks:
//   overview - what is happening right now?
//   operate  - what do I do with my sandboxes, models and data day to day?
//   setup    - what do I connect, build or configure once?
//   security - what may the sandboxes reach, and who decided that?
//   admin    - who may use this, and what is the platform doing?
//   help     - how does this work?
//
// A route with `action` does not open a page: `dock` opens the dock on the tab named by the
// route id, `shortcuts` opens the shortcut sheet.

/** Group headings in display order. */
export const GROUPS = [
  { id: 'overview', label: 'Overview' },
  { id: 'operate', label: 'Operate' },
  { id: 'setup', label: 'Setup' },
  { id: 'security', label: 'Security' },
  { id: 'admin', label: 'Administration' },
  { id: 'help', label: 'Help' },
];

/**
 * Every route. `minRole` hides an entry from a role below it; the API still
 * enforces the same rule, so hiding is convenience and not protection.
 */
export const ROUTES = [
  { id: 'home', label: 'Home', group: 'overview' },
  { id: 'live', label: 'Agent situation', group: 'overview' },
  { id: 'traffic', label: 'Live traffic', group: 'overview' },
  { id: 'logs', label: 'Live logs', group: 'overview' },
  { id: 'alerts', label: 'Alerts', group: 'overview', action: 'dock' },

  // A member's day starts here: their own sandboxes, one click into the workbox.
  { id: 'my', label: 'My sandboxes', group: 'operate' },
  { id: 'sandboxes', label: 'Sandboxes', group: 'operate' },
  { id: 'models', label: 'Models', group: 'operate' },
  { id: 'clusters', label: 'Model clusters', group: 'operate' },
  { id: 'moves', label: 'Moves', group: 'operate' },
  { id: 'persistent', label: 'Persistent data', group: 'operate' },
  { id: 'operations', label: 'Tasks', group: 'operate' },

  { id: 'tenants', label: 'Data centers & sites', group: 'setup' },
  { id: 'nodes', label: 'Nodes', group: 'setup' },
  { id: 'hosting', label: 'Rented machines', group: 'setup', minRole: 'operator' },
  { id: 'images', label: 'Images', group: 'setup' },
  { id: 'providers', label: 'Providers & keys', group: 'setup', minRole: 'operator' },
  { id: 'mcp', label: 'MCP servers', group: 'setup' },
  { id: 'media', label: 'Media', group: 'setup' },
  { id: 'skills', label: 'Skills', group: 'setup' },
  { id: 'cudax', label: 'CUDA-X', group: 'setup' },
  { id: 'builder', label: 'Build image', group: 'setup' },
  { id: 'recipes', label: 'Recipes', group: 'setup' },
  { id: 'proxies', label: 'Exit proxies', group: 'setup' },

  { id: 'policies', label: 'Policies', group: 'security' },
  { id: 'privacy', label: 'Content privacy', group: 'security' },
  { id: 'master', label: 'Master control', group: 'security' },
  { id: 'usage', label: 'Usage & cost', group: 'security' },

  // Only an admin manages who may sign in; the page and the API both refuse anyone else.
  { id: 'users', label: 'Users & permissions', group: 'admin', minRole: 'admin' },
  { id: 'system', label: 'System containers', group: 'admin', minRole: 'operator' },
  { id: 'cluster', label: 'Cluster health', group: 'admin', minRole: 'operator' },

  { id: 'docs', label: 'Docs', group: 'help' },
  { id: 'shortcuts', label: 'Shortcuts', group: 'help', action: 'shortcuts' },
];

// `member` is the employee role (ADR 0045): everything inside their own
// sandboxes, nothing that belongs to the environment. Without it in the table a
// member ranked below a viewer and saw less than they may do.
export const RANK = { viewer: 1, member: 2, operator: 3, admin: 4 };

/** True when `role` may see `route`. An unknown role sees only unrestricted entries. */
export function visible(route, role) {
  if (!route.minRole) return true;
  return (RANK[role] ?? 0) >= RANK[route.minRole];
}

/** The menu for a role: groups in order, each with its visible routes, empty groups dropped. */
export function menuFor(role) {
  return GROUPS.map((group) => ({
    ...group,
    routes: ROUTES.filter((route) => route.group === group.id && visible(route, role)),
  })).filter((group) => group.routes.length > 0);
}
