// HelpDrawer: opened by `ui.help.open('slug#heading')` (HelpTip, the ? button). Shows the
// anchored document section first, a "Can't find it?" search over docs, menu and inventory,
// a link to the full docs and a button to restart the first-run tour. Mounted once by the Shell.
import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js';
import '../../styles/help.css';
import Drawer from '../../components/Drawer.jsx';
import { useUi, onKey } from '../../state/ui.jsx';
import { useSession } from '../../state/session.jsx';
import { menuFor } from '../../navigation.js';
import { navigate, paths } from '../../routing.js';
import * as searchApi from '../../api/search.js';
import { search as searchDocs, splitAnchor } from './docsSearch.js';
import { interceptDocLinks, restartTour, scrollToHeading, trapFocus, useDoc, useIndex } from './useHelp.js';

export default function HelpDrawer() {
  const ui = useUi();
  const session = useSession();
  const { index } = useIndex();
  const [slug, setSlug] = createSignal(null);
  const [heading, setHeading] = createSignal(null);
  const [query, setQuery] = createSignal('');
  const [results, setResults] = createSignal([]);
  const [searching, setSearching] = createSignal(false);
  const { doc, error, loading } = useDoc(slug);
  let body;
  let root;

  // Each opening starts at its anchor; without one, at the first document of the index.
  createEffect(() => {
    if (!ui.help.isOpen()) return;
    const anchor = splitAnchor(ui.help.anchor());
    setSlug(anchor.slug || index()[0]?.slug || null);
    setHeading(anchor.heading);
  });
  createEffect(() => {
    if (doc() && body) {
      if (heading()) scrollToHeading(body, heading());
      else body.scrollTop = 0;
    }
  });
  // Escape closes; Tab stays inside the drawer.
  createEffect(() => {
    if (!ui.help.isOpen()) return;
    const stopEscape = onKey('Escape', () => ui.help.close());
    const stopTrap = root ? trapFocus(root) : () => {};
    onCleanup(() => {
      stopEscape();
      stopTrap();
    });
  });

  // Search docs and menu at once, the inventory after a short pause.
  let timer = null;
  let ticket = 0;
  onCleanup(() => clearTimeout(timer));
  function onQuery(value) {
    setQuery(value);
    clearTimeout(timer);
    ticket += 1;
    const mine = ticket;
    const q = value.trim();
    if (!q) {
      setResults([]);
      setSearching(false);
      return;
    }
    const lower = q.toLowerCase();
    const docHits = searchDocs(index(), q).slice(0, 6).map((hit) => ({
      key: `doc:${hit.slug}:${hit.headingId || ''}`,
      group: 'Docs',
      label: hit.heading ? `${hit.title} › ${hit.heading}` : hit.title,
      open: () => { setSlug(hit.slug); setHeading(hit.headingId || null); },
    }));
    const menuHits = menuFor(session.identity()?.role)
      .flatMap((group) => group.routes)
      .filter((route) => route.label.toLowerCase().includes(lower))
      .slice(0, 5)
      .map((route) => ({ key: `menu:${route.id}`, group: 'Menu', label: route.label, open: () => openMenu(route) }));
    setResults([...docHits, ...menuHits]);
    setSearching(true);
    timer = setTimeout(async () => {
      try {
        const found = await searchApi.search({ q, limit: 6 });
        if (mine !== ticket) return;
        const items = (found?.items || []).map((item) => ({
          key: `inv:${item.kind}:${item.id}`,
          group: 'Inventory',
          label: `${item.name} (${item.kind})`,
          open: () => { ui.help.close(); navigate(item.path); },
        }));
        setResults((list) => [...list, ...items]);
      } catch {
        // The inventory search is a bonus; docs and menu results stay.
      } finally {
        if (mine === ticket) setSearching(false);
      }
    }, 300);
  }

  // A menu route opens a page, or the dock, or the shortcut sheet.
  function openMenu(route) {
    ui.help.close();
    if (route.action === 'shortcuts') ui.shortcuts.open();
    else if (route.action === 'dock') {
      ui.dock.setTab(route.id);
      ui.dock.setOpen(true);
    } else navigate(paths.menu(route.id));
  }

  function openFull() {
    const current = slug();
    ui.help.close();
    navigate(paths.docs(current));
  }

  return (
    <Drawer open={ui.help.isOpen()} title="Help" subtitle={doc()?.meta.title || null} onClose={() => ui.help.close()}>
      <div ref={root} class="help-drawer">
        <label class="docs-search">
          <span class="eyebrow">Can't find it?</span>
          <input type="search" value={query()} placeholder="Search docs, menu and your inventory" onInput={(event) => onQuery(event.currentTarget.value)} />
        </label>
        <Show when={query().trim()}>
          <ul class="help-results" aria-label="Search results">
            <For each={results()} fallback={<li class="muted">{searching() ? 'Searching…' : 'Nothing found.'}</li>}>
              {(result) => (
                <li>
                  <span class="badge muted">{result.group}</span>
                  <span>{result.label}</span>
                  <button class="tiny secondary" onClick={() => { result.open(); setQuery(''); setResults([]); }}>Open</button>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <div ref={body} class="md help-doc" onClick={(event) => interceptDocLinks(event, (nextSlug, nextHeading) => { if (nextSlug) setSlug(nextSlug); setHeading(nextHeading); if (!nextSlug && nextHeading) scrollToHeading(body, nextHeading); })} innerHTML={doc()?.html || ''} />
        <Show when={loading() && !doc()}><p class="muted">Loading…</p></Show>
        <Show when={error()}><p class="notice">This document could not be loaded: {error()}</p></Show>
        <Show when={!slug() && !loading() && !error()}><p class="muted">There are no help documents yet.</p></Show>
        <footer class="help-footer">
          <button class="secondary" onClick={openFull} disabled={!slug()}>Open full docs</button>
          <button class="ghost" onClick={() => { ui.help.close(); restartTour(); }}>Restart tour</button>
          <button class="ghost" onClick={() => { ui.help.close(); ui.shortcuts.open(); }}>Keyboard shortcuts</button>
        </footer>
      </div>
    </Drawer>
  );
}
