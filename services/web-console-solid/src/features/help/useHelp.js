// Shared logic of the help features: cached index and documents, scrolling to a heading,
// a focus trap for dialogs, the global shortcuts and the "restart tour" request.
import { createEffect, createSignal, onCleanup } from 'solid-js';
import * as helpApi from '../../api/help.js';
import { parseFrontMatter, render, headings } from '../../components/markdown.js';
import { onKey } from '../../state/ui.jsx';
import { navigate, paths } from '../../routing.js';

let indexPromise = null;
const docPromises = new Map();

// The help index, fetched once per page load; a failure is not cached.
export function loadIndex() {
  if (!indexPromise) {
    indexPromise = helpApi.index().catch((error) => {
      indexPromise = null;
      throw error;
    });
  }
  return indexPromise;
}

// One parsed document `{ meta, html, headings }`, fetched once per page load.
export function loadDoc(slug) {
  if (!docPromises.has(slug)) {
    docPromises.set(
      slug,
      helpApi
        .doc(slug)
        .then((text) => {
          const { meta, body } = parseFrontMatter(text);
          return { meta, html: render(body), headings: headings(body) };
        })
        .catch((error) => {
          docPromises.delete(slug);
          throw error;
        }),
    );
  }
  return docPromises.get(slug);
}

// The index as signals: `{ index, error }`.
export function useIndex() {
  const [index, setIndex] = createSignal([]);
  const [error, setError] = createSignal(null);
  loadIndex().then(setIndex, (failure) => setError(failure.message));
  return { index, error };
}

// One document as signals for the slug returned by `slug()`: `{ doc, error, loading }`.
export function useDoc(slug) {
  const [doc, setDoc] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [loading, setLoading] = createSignal(false);
  let ticket = 0;
  createEffect(() => {
    const wanted = slug();
    ticket += 1;
    const mine = ticket;
    if (!wanted) {
      setDoc(null);
      setError(null);
      setLoading(false);
      return;
    }
    setLoading(true);
    loadDoc(wanted).then(
      (result) => {
        if (mine !== ticket) return;
        setDoc(result);
        setError(null);
        setLoading(false);
      },
      (failure) => {
        if (mine !== ticket) return;
        setDoc(null);
        setError(failure.message);
        setLoading(false);
      },
    );
  });
  return { doc, error, loading };
}

// The heading id in the address bar (`/docs/x#heading`), or null. Read only to deep-link.
export function currentHash() {
  const hash = typeof window === 'undefined' ? '' : window.location.hash;
  return hash.length > 1 ? decodeURIComponent(hash.slice(1)) : null;
}

// Scroll the heading with `id` into view inside `root`; true when it exists.
export function scrollToHeading(root, id) {
  if (!root || !id) return false;
  const target = root.querySelector(`[id="${String(id).replace(/"/g, '')}"]`);
  if (!target) return false;
  target.scrollIntoView({ block: 'start' });
  return true;
}

// Click handler for rendered markdown: an internal `/docs/<slug>#<heading>` link calls
// `open(slug, heading)` instead of reloading the page. Other links behave as usual.
export function interceptDocLinks(event, open) {
  const link = event.target.closest && event.target.closest('a[href]');
  if (!link || event.defaultPrevented || event.metaKey || event.ctrlKey || event.shiftKey) return;
  const href = link.getAttribute('href') || '';
  const match = /^\/docs\/([A-Za-z0-9_-]{1,100})(?:#(.+))?$/.exec(href);
  if (match) {
    event.preventDefault();
    open(match[1], match[2] || null);
  } else if (href.startsWith('#')) {
    event.preventDefault();
    open(null, href.slice(1));
  }
}

// Keep Tab inside `root` while it is mounted; returns the stop function. The caller closes
// on Escape. Focus moves into `root` first.
export function trapFocus(root) {
  const selector = 'a[href], button:not([disabled]), input:not([disabled]), select, textarea, [tabindex]:not([tabindex="-1"])';
  const previous = document.activeElement;
  const first = root.querySelector(selector);
  if (first) first.focus();
  else {
    root.setAttribute('tabindex', '-1');
    root.focus();
  }
  function onTab(event) {
    if (event.key !== 'Tab') return;
    const items = [...root.querySelectorAll(selector)].filter((item) => item.offsetParent !== null);
    if (items.length === 0) {
      event.preventDefault();
      return;
    }
    const head = items[0];
    const tail = items[items.length - 1];
    if (event.shiftKey && document.activeElement === head) {
      event.preventDefault();
      tail.focus();
    } else if (!event.shiftKey && document.activeElement === tail) {
      event.preventDefault();
      head.focus();
    } else if (!root.contains(document.activeElement)) {
      event.preventDefault();
      head.focus();
    }
  }
  document.addEventListener('keydown', onTab);
  return () => {
    document.removeEventListener('keydown', onTab);
    if (previous && typeof previous.focus === 'function') previous.focus();
  };
}

// The shortcuts shown on the sheet, in groups. `combo` is the `onKey` syntax.
export const SHORTCUTS = [
  { group: 'General', items: [
    { combo: 'mod+k', label: 'Open the command palette' },
    { combo: '?', label: 'Show this sheet' },
    { combo: 'd', label: 'Open or close the dock' },
    { combo: '[', label: 'Previous tab' },
    { combo: ']', label: 'Next tab' },
    { combo: 'Escape', label: 'Close a dialog, drawer or menu' },
  ] },
  { group: 'Go to', items: [
    { combo: 'g h', label: 'Home' },
    { combo: 'g s', label: 'Sandboxes' },
    { combo: 'g m', label: 'Models' },
    { combo: 'g n', label: 'Nodes' },
    { combo: 'g t', label: 'Live traffic' },
    { combo: 'g l', label: 'Live logs' },
  ] },
];

const GO_TO = { 'g h': paths.home(), 'g s': paths.menu('sandboxes'), 'g m': paths.menu('models'), 'g n': paths.menu('nodes'), 'g t': paths.menu('traffic'), 'g l': paths.menu('logs') };

// Register `?`, `d` and the `g x` jumps. Two-key sequences are supported by `onKey` itself
// (800 ms window). `mod+k` belongs to the palette host; `[` and `]` to the page with tabs.
export function registerGlobalShortcuts(ui) {
  const stops = [
    onKey('?', () => ui.shortcuts.open()),
    onKey('d', () => ui.dock.setOpen(!ui.dock.open())),
    ...Object.entries(GO_TO).map(([combo, path]) => onKey(combo, () => navigate(path))),
  ];
  onCleanup(() => stops.forEach((stop) => stop()));
}

const [tourRuns, setTourRuns] = createSignal(0);

// How many times a tour restart was requested; the tour reacts to changes.
export const tourRequests = tourRuns;

// Ask the tour to run again from the first step.
export function restartTour() {
  setTourRuns((count) => count + 1);
}
