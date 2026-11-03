// The UI preferences that survive a reload, as pure functions (no DOM, no Solid).
//
// Only presentation lives here: sidebar width, dock state, tree view, the last console choice
// per sandbox, log panes, palette recents and dismissed tours. Never a token, never a secret.

/** The localStorage key. */
export const STORAGE_KEY = 'nestbox:ui';

/** What a fresh browser starts with. */
export const DEFAULTS = Object.freeze({
  sidebarCollapsed: false,
  sidebarWidth: 272,
  dockOpen: false,
  dockTab: 'tasks',
  dockHeight: 220,
  treeView: 'infrastructure',
  consoleMode: {},
  logPanes: [],
  paletteRecent: [],
  toursDismissed: [],
});

const TREE_VIEWS = ['infrastructure', 'workloads', 'models'];
const DOCK_TABS = ['tasks', 'events', 'alerts'];

const clamp = (value, low, high, fallback) =>
  typeof value === 'number' && Number.isFinite(value) ? Math.min(high, Math.max(low, value)) : fallback;

/** Turn anything parsed from storage into a complete, well-typed preference object. */
export function sanitize(raw) {
  const input = raw && typeof raw === 'object' ? raw : {};
  const consoleMode = {};
  if (input.consoleMode && typeof input.consoleMode === 'object') {
    for (const [id, mode] of Object.entries(input.consoleMode)) {
      if (typeof mode === 'string' && mode.length <= 20) consoleMode[id] = mode;
    }
  }
  const strings = (list, max) =>
    Array.isArray(list) ? list.filter((item) => typeof item === 'string').slice(0, max) : [];
  return {
    sidebarCollapsed: input.sidebarCollapsed === true,
    sidebarWidth: clamp(input.sidebarWidth, 200, 480, DEFAULTS.sidebarWidth),
    dockOpen: input.dockOpen === true,
    dockTab: DOCK_TABS.includes(input.dockTab) ? input.dockTab : DEFAULTS.dockTab,
    dockHeight: clamp(input.dockHeight, 120, 600, DEFAULTS.dockHeight),
    treeView: TREE_VIEWS.includes(input.treeView) ? input.treeView : DEFAULTS.treeView,
    consoleMode,
    logPanes: strings(input.logPanes, 8),
    paletteRecent: Array.isArray(input.paletteRecent)
      ? input.paletteRecent.filter((item) => item && typeof item.id === 'string').slice(0, 8)
      : [],
    toursDismissed: strings(input.toursDismissed, 20),
  };
}

/** Read preferences from stored text; garbage gives the defaults. */
export function parse(text) {
  if (!text) return sanitize(null);
  try {
    return sanitize(JSON.parse(text));
  } catch {
    return sanitize(null);
  }
}

/** The text to store. */
export function serialize(prefs) {
  return JSON.stringify(sanitize(prefs));
}
