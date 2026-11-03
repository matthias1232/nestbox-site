// UI preferences and small shared UI state (contract C16, pattern J7).
//
// Persisted under localStorage['nestbox:ui'], and only presentation: sidebar, dock, tree view,
// last console choice per sandbox, log panes, palette recents, dismissed tours. Ignored alerts,
// selections and toasts live in memory only.

import { createContext, createEffect, createSignal, onCleanup, useContext } from 'solid-js';
import { STORAGE_KEY, parse, serialize } from './uiPrefs.js';
import { createMatcher, isTypingTarget } from './shortcuts.js';

const UiContext = createContext();

const IS_MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || '');

// ---- keyboard registry: one window listener for every shortcut --------------------------

const keyHandlers = new Set();
let keyListening = false;

function onWindowKey(event) {
  const typing = isTypingTarget(event.target);
  for (const entry of [...keyHandlers]) {
    // A shortcut never fires while the user types, except Escape.
    if (typing && entry.combo !== 'Escape') continue;
    if (entry.matcher.feed(event)) {
      entry.handler(event);
    }
  }
}

// Register a shortcut such as `mod+k`, `?`, `Escape` or `g h`. Returns the stop function.
export function onKey(combo, handler) {
  const entry = { combo, handler, matcher: createMatcher(combo, { isMac: IS_MAC }) };
  keyHandlers.add(entry);
  if (!keyListening && typeof window !== 'undefined') {
    window.addEventListener('keydown', onWindowKey);
    keyListening = true;
  }
  return () => {
    keyHandlers.delete(entry);
    if (keyHandlers.size === 0 && keyListening) {
      window.removeEventListener('keydown', onWindowKey);
      keyListening = false;
    }
  };
}

function readStored() {
  try {
    return parse(typeof localStorage === 'undefined' ? null : localStorage.getItem(STORAGE_KEY));
  } catch {
    return parse(null);
  }
}

// Provide `useUi()` to the tree.
export function UiProvider(props) {
  const initial = readStored();

  const [sidebarCollapsed, setSidebarCollapsed] = createSignal(initial.sidebarCollapsed);
  const [sidebarWidth, setSidebarWidth] = createSignal(initial.sidebarWidth);
  const [dockOpen, setDockOpen] = createSignal(initial.dockOpen);
  const [dockTab, setDockTab] = createSignal(initial.dockTab);
  const [dockHeight, setDockHeight] = createSignal(initial.dockHeight);
  const [treeView, setTreeView] = createSignal(initial.treeView);
  const [consoleModes, setConsoleModes] = createSignal(initial.consoleMode);
  const [logPanes, setLogPanes] = createSignal(initial.logPanes);
  const [recent, setRecent] = createSignal(initial.paletteRecent);

  const [selectionKind, setSelectionKind] = createSignal(null);
  const [selectionIds, setSelectionIds] = createSignal(new Set());
  const [toasts, setToasts] = createSignal([]);
  const [paletteOpen, setPaletteOpen] = createSignal(false);
  const [helpAnchor, setHelpAnchor] = createSignal(null);
  const [helpOpen, setHelpOpen] = createSignal(false);
  const [shortcutsOpen, setShortcutsOpen] = createSignal(false);
  const [alertCount, setAlertCount] = createSignal(0);
  const [ignored, setIgnored] = createSignal(new Set());

  // Write the preferences whenever one of them changes.
  createEffect(() => {
    const text = serialize({
      sidebarCollapsed: sidebarCollapsed(),
      sidebarWidth: sidebarWidth(),
      dockOpen: dockOpen(),
      dockTab: dockTab(),
      dockHeight: dockHeight(),
      treeView: treeView(),
      consoleMode: consoleModes(),
      logPanes: logPanes(),
      paletteRecent: recent(),
      toursDismissed: initial.toursDismissed,
    });
    try {
      localStorage.setItem(STORAGE_KEY, text);
    } catch {
      // A full or blocked storage must not break the console.
    }
  });

  const timers = new Set();
  onCleanup(() => timers.forEach((timer) => clearTimeout(timer)));
  let toastSeq = 0;

  const ui = {
    sidebar: {
      collapsed: sidebarCollapsed,
      setCollapsed: setSidebarCollapsed,
      width: sidebarWidth,
      setWidth: setSidebarWidth,
    },
    dock: {
      open: dockOpen,
      setOpen: setDockOpen,
      tab: dockTab,
      setTab: setDockTab,
      height: dockHeight,
      setHeight: setDockHeight,
    },
    tree: { view: treeView, setView: setTreeView },
    selection: {
      ids: selectionIds,
      kind: selectionKind,
      // Select or deselect one id; selecting another kind starts a new selection.
      toggle(kind, id) {
        const next = new Set(selectionKind() === kind ? selectionIds() : []);
        if (next.has(id)) next.delete(id);
        else next.add(id);
        setSelectionKind(next.size ? kind : null);
        setSelectionIds(next);
      },
      clear() {
        setSelectionKind(null);
        setSelectionIds(new Set());
      },
      set(kind, ids) {
        setSelectionKind(ids.length ? kind : null);
        setSelectionIds(new Set(ids));
      },
    },
    toast: {
      queue: toasts,
      // Show a toast; `undo` is an optional function the Undo button calls. Returns its id.
      show({ tone = 'info', text, undo = null, ms = 6000 }) {
        toastSeq += 1;
        const id = toastSeq;
        setToasts((list) => [...list, { id, tone, text, undo }].slice(-5));
        if (ms > 0) {
          const timer = setTimeout(() => {
            timers.delete(timer);
            ui.toast.dismiss(id);
          }, ms);
          timers.add(timer);
        }
        return id;
      },
      dismiss(id) {
        setToasts((list) => list.filter((toast) => toast.id !== id));
      },
    },
    palette: {
      isOpen: paletteOpen,
      open: () => setPaletteOpen(true),
      close: () => setPaletteOpen(false),
      recent,
      // Remember an opened result (`{ id, label, path }`), newest first, at most eight.
      addRecent(item) {
        setRecent((list) => [item, ...list.filter((entry) => entry.id !== item.id)].slice(0, 8));
      },
    },
    help: {
      isOpen: helpOpen,
      anchor: helpAnchor,
      // Open the help drawer, optionally at `slug#heading`.
      open(anchor = null) {
        setHelpAnchor(anchor);
        setHelpOpen(true);
      },
      close: () => setHelpOpen(false),
    },
    shortcuts: {
      isOpen: shortcutsOpen,
      open: () => setShortcutsOpen(true),
      close: () => setShortcutsOpen(false),
    },
    alerts: {
      count: alertCount,
      setCount: setAlertCount,
      // Ignored alerts last until the page reloads; they are never persisted.
      ignore(id) {
        setIgnored((set) => new Set(set).add(id));
      },
      isIgnored: (id) => ignored().has(id),
    },
    console: {
      mode: (sandboxId) => consoleModes()[sandboxId] || null,
      setMode(sandboxId, mode) {
        setConsoleModes((all) => ({ ...all, [sandboxId]: mode }));
      },
    },
    logs: { panes: logPanes, setPanes: setLogPanes },
  };

  return <UiContext.Provider value={ui}>{props.children}</UiContext.Provider>;
}

// The UI store of the current app.
export function useUi() {
  return useContext(UiContext);
}
