// The ordered tab list of the Admin Center, pure data so it can be tested without a DOM.
//
// `component` is a key that AdminCenter.jsx maps to a real component; keeping names here and
// components there is what lets this file be tested with plain `node --test`.

import { RANK } from '../../navigation.js';

/** Every tab, in the order an operator asks the questions. */
export const ADMIN_TABS = [
  { id: 'summary', label: 'Summary', minRole: 'member', component: 'summary' },
  { id: 'live', label: 'Live', minRole: 'member', component: 'live' },
  { id: 'traffic', label: 'Traffic', minRole: 'member', component: 'traffic' },
  { id: 'policies', label: 'Policies & DNS', minRole: 'member', component: 'policies' },
  { id: 'mounts', label: 'Mounts', minRole: 'member', component: 'mounts' },
  { id: 'console', label: 'Console', minRole: 'member', component: 'console' },
  { id: 'access', label: 'Access', minRole: 'member', component: 'access' },
  { id: 'mcp', label: 'MCP', minRole: 'member', component: 'mcp' },
  { id: 'agents', label: 'Agents', minRole: 'member', component: 'agents' },
  { id: 'processes', label: 'Processes', minRole: 'operator', component: 'processes' },
  { id: 'inventory', label: 'Inventory', minRole: 'operator', component: 'inventory' },
  { id: 'logs', label: 'Logs', minRole: 'member', component: 'logs' },
  { id: 'privacy', label: 'Privacy / DLP', minRole: 'member', component: 'privacy' },
  { id: 'usage', label: 'Usage', minRole: 'member', component: 'usage' },
  { id: 'resources', label: 'Resources', minRole: 'operator', component: 'resources' },
  { id: 'sessions', label: 'Sessions', minRole: 'operator', component: 'sessions' },
  { id: 'supervisor', label: 'Supervisor', minRole: 'operator', component: 'supervisor' },
  { id: 'map', label: 'Topology', minRole: 'operator', component: 'map' },
  { id: 'permissions', label: 'Permissions', minRole: 'admin', component: 'permissions' },
];

/** Old tab ids (bookmarks, links from other pages) and the tab that now holds them. */
export const TAB_ALIASES = {
  dashboard: 'summary',
  connections: 'live',
  commands: 'live',
  chats: 'live',
  terminal: 'console',
  desktop: 'console',
};

/** The tabs a role may see, in order. An unknown role sees only the member tabs. */
export function tabsFor(role) {
  const rank = RANK[role] ?? RANK.member;
  return ADMIN_TABS.filter((tab) => rank >= RANK[tab.minRole]);
}

/** The tab id to show for a requested id: aliases resolved, hidden or unknown ids fall back to Summary. */
export function resolveTab(requested, role) {
  const id = TAB_ALIASES[requested] || requested;
  return tabsFor(role).some((tab) => tab.id === id) ? id : 'summary';
}

/** For an alias that names one half of a grouped tab, the sub-view it should open (else null). */
export function subViewFor(requested) {
  if (requested === 'terminal') return 'shell';
  if (requested === 'desktop') return 'desktop';
  if (requested === 'connections' || requested === 'commands' || requested === 'chats') return requested;
  return null;
}
