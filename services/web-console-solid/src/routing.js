// Path-based navigation without a router library (contract C1).
//
// The console keeps no view in a signal: the address bar is the single source of truth, so
// the back button, a reload and a shared link all land on the same screen. `parsePath` says
// what an address means, `paths` builds addresses, and a unit test proves both.

import { ROUTES } from './navigation.js';

/** The address of one sandbox's Workbox. */
export const boxPath = (id) => `/box/${id}`;
/** The address of the list of my sandboxes. */
export const myPath = () => '/my';
/** The address of one terminal tab in its own window. */
export const terminalPath = (id, tab) => `/box/${id}/terminal/${tab}`;
/** The address of the desktop in its own window. */
export const desktopPath = (id) => `/box/${id}/desktop`;

/** Every address the console builds, by name. */
export const paths = {
  home: () => '/',
  dc: (id) => `/dc/${id}`,
  site: (id) => `/site/${id}`,
  node: (id) => `/node/${id}`,
  sandbox: (id, tab) => (tab ? `/sandboxes/${id}?tab=${encodeURIComponent(tab)}` : `/sandboxes/${id}`),
  box: boxPath,
  my: myPath,
  model: (key) => `/models/${encodeURIComponent(key)}`,
  cluster: (id) => `/clusters/${id}`,
  docs: (slug) => (slug ? `/docs/${slug}` : '/docs'),
  popLogs: (id) => `/pop/logs/${id || 'all'}`,
  popTraffic: () => '/pop/traffic',
  // A menu id such as `traffic` or `policies`; `home`, `my` and `docs` have their own addresses.
  menu: (id) => {
    if (id === 'home') return '/';
    if (id === 'my') return '/my';
    if (id === 'docs') return '/docs';
    return `/${id}`;
  },
};

/** Push a path and let the shell re-read it, like a link followed. */
export function navigate(path) {
  if (window.location.pathname + window.location.search === path) return;
  window.history.pushState({}, '', path);
  window.dispatchEvent(new PopStateEvent('popstate'));
}

const UUID = '([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})';
const MENU_IDS = new Set(ROUTES.map((route) => route.id));

// Order matters: the longer, more specific patterns come first.
const PATTERNS = [
  { re: new RegExp(`^/box/${UUID}/terminal/([A-Za-z0-9_-]{1,64})/?$`), make: (m) => ({ kind: 'terminal', id: m[1], tab: m[2] }) },
  { re: new RegExp(`^/box/${UUID}/desktop/?$`), make: (m) => ({ kind: 'desktop', id: m[1] }) },
  { re: new RegExp(`^/box/${UUID}/?$`), make: (m) => ({ kind: 'box', id: m[1] }) },
  { re: /^\/my\/?$/, make: () => ({ kind: 'my' }) },
  { re: new RegExp(`^/dc/${UUID}/?$`), make: (m) => ({ kind: 'dc', id: m[1] }) },
  { re: new RegExp(`^/site/${UUID}/?$`), make: (m) => ({ kind: 'site', id: m[1] }) },
  { re: new RegExp(`^/node/${UUID}/?$`), make: (m) => ({ kind: 'node', id: m[1] }) },
  { re: new RegExp(`^/clusters/${UUID}/?$`), make: (m) => ({ kind: 'cluster', id: m[1] }) },
  { re: /^\/models\/([^/]{1,200})\/?$/, make: (m) => ({ kind: 'model', id: decodeURIComponent(m[1]) }) },
  { re: /^\/docs\/([A-Za-z0-9_-]{1,100})\/?$/, make: (m) => ({ kind: 'docs', slug: m[1] }) },
  { re: /^\/docs\/?$/, make: () => ({ kind: 'docs', slug: null }) },
  { re: /^\/pop\/logs\/([0-9a-fA-F-]{36}|all)\/?$/, make: (m) => ({ kind: 'poplogs', id: m[1] }) },
  { re: /^\/pop\/traffic\/?$/, make: () => ({ kind: 'poptraffic' }) },
];

/** Kinds that render without the shell: pop-out windows. */
export const SHELLLESS = new Set(['terminal', 'desktop', 'poplogs', 'poptraffic']);

/**
 * What an address asks for: `{ kind, ... }` as in contract C1.
 *
 * `search` is the query string (`?tab=policies`); only the Admin Center reads it.
 * Anything unknown is the home page.
 */
export function parsePath(pathname, search = '') {
  if (pathname === '/' || pathname === '') return { kind: 'home' };
  const admin = new RegExp(`^/sandboxes/${UUID}/?$`).exec(pathname);
  if (admin) {
    const tab = new URLSearchParams(search).get('tab');
    return { kind: 'admin', id: admin[1], tab: tab || null };
  }
  for (const pattern of PATTERNS) {
    const match = pattern.re.exec(pathname);
    if (match) return pattern.make(match);
  }
  const menu = /^\/([a-z][a-z-]{0,40})\/?$/.exec(pathname);
  if (menu && MENU_IDS.has(menu[1]) && menu[1] !== 'home') return { kind: 'menu', id: menu[1] };
  return { kind: 'home' };
}
