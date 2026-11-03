// The command palette: one dialog that searches the server, the menu and the help documents.
//
// Props (contract C15): `open`, `onClose`, `search(params) -> Promise`, `docsIndex`, `onOpen(path)`.
// Added without removing anything: `environments` ([{ id, name }]) for the Data center chip,
// `menu` (entries from `menuEntries`) and `recent` (the persisted recents). It fetches nothing
// itself; the host hands in the search function.
import { For, Show, createEffect, createMemo, createSignal, on, onCleanup } from 'solid-js';
import StatusBadge from './StatusBadge.jsx';
import {
  KINDS,
  KIND_LABELS,
  STATUSES,
  applyFilters,
  createLatest,
  flatten,
  matchDocs,
  matchMenu,
  mergeResults,
  moveIndex,
  searchParams,
  serverItems,
} from './palette.js';
import '../styles/palette.css';

const DEBOUNCE_MS = 200;
const FOCUSABLE = 'button:not([disabled]), input, select, a[href], [tabindex]:not([tabindex="-1"])';

// Two letters that stand for a kind where an icon would go; the word is also in the subtitle.
const GLYPHS = {
  sandbox: 'SB', node: 'ND', model: 'ML', cluster: 'CL', policy: 'PL', user: 'US',
  site: 'ST', data_center: 'DC', job: 'JB', doc: 'DO', menu: 'MN',
};

export default function CommandPalette(props) {
  const [q, setQ] = createSignal('');
  const [kind, setKind] = createSignal('');
  const [environment, setEnvironment] = createSignal('');
  const [status, setStatus] = createSignal('');
  const [active, setActive] = createSignal(0);
  const [server, setServer] = createSignal([]);
  const [loading, setLoading] = createSignal(false);
  const [failure, setFailure] = createSignal('');
  const latest = createLatest();
  let box;
  let input;
  let before = null;

  const filters = () => ({ kind: kind(), environment: environment(), status: status() });

  // Debounced server search; an older answer is dropped when a newer request started.
  createEffect(() => {
    const text = q().trim();
    const chips = filters();
    if (!props.open) return;
    if (!text) {
      latest.begin();
      setServer([]);
      setLoading(false);
      setFailure('');
      return;
    }
    const timer = setTimeout(async () => {
      const id = latest.begin();
      setLoading(true);
      try {
        const answer = await props.search(searchParams(text, chips));
        if (!latest.isCurrent(id)) return;
        setServer(serverItems(answer?.items));
        setFailure('');
      } catch (error) {
        if (!latest.isCurrent(id)) return;
        setServer([]);
        setFailure(error?.message || 'Search failed');
      } finally {
        if (latest.isCurrent(id)) setLoading(false);
      }
    }, DEBOUNCE_MS);
    onCleanup(() => clearTimeout(timer));
  });

  const groups = createMemo(() => {
    const text = q();
    const merged = mergeResults(
      server(),
      matchMenu(props.menu || [], text),
      matchDocs(props.docsIndex || [], text),
      props.recent || [],
      text,
    );
    return applyFilters(merged, filters());
  });
  const flat = createMemo(() => flatten(groups()));

  // A new query or chip puts the cursor back on the first row.
  createEffect(on([q, kind, environment, status], () => setActive(0), { defer: true }));
  createEffect(() => {
    if (active() >= flat().length) setActive(0);
  });

  // Reset and focus when it opens; give focus back when it closes.
  createEffect(
    on(
      () => props.open,
      (open) => {
        if (open) {
          before = document.activeElement;
          setQ('');
          setKind('');
          setEnvironment('');
          setStatus('');
          setActive(0);
          setServer([]);
          setFailure('');
          queueMicrotask(() => input?.focus());
        } else if (before && typeof before.focus === 'function') {
          before.focus();
          before = null;
        }
      },
    ),
  );

  // Keep the active row visible.
  createEffect(() => {
    const row = flat()[active()];
    if (!row || !box) return;
    box.querySelector(`[data-row-id="${CSS.escape(row.id)}"]`)?.scrollIntoView?.({ block: 'nearest' });
  });

  function choose(item) {
    if (!item) return;
    props.onOpen(item.path, item);
  }

  function onKeyDown(event) {
    if (event.key === 'Escape') {
      event.preventDefault();
      props.onClose();
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      setActive(moveIndex(active(), event.key === 'ArrowDown' ? 1 : -1, flat().length));
    } else if (event.key === 'Enter' && event.target === input) {
      event.preventDefault();
      choose(flat()[active()]);
    } else if (event.key === 'Tab') {
      // Trap focus: Tab cycles input and chips inside the dialog.
      const items = [...box.querySelectorAll(FOCUSABLE)].filter((el) => el.tabIndex >= 0);
      if (!items.length) return;
      const first = items[0];
      const last = items[items.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }
  }

  const rowId = (item) => `palette-row-${item.id.replace(/[^A-Za-z0-9_-]/g, '_')}`;

  return (
    <Show when={props.open}>
      <div class="palette-backdrop" onMouseDown={(event) => event.target === event.currentTarget && props.onClose()}>
        <div
          class="palette"
          role="dialog"
          aria-modal="true"
          aria-label="Search"
          ref={box}
          onKeyDown={onKeyDown}
        >
          <input
            ref={input}
            class="palette-input"
            type="text"
            role="combobox"
            aria-expanded="true"
            aria-controls="palette-list"
            aria-activedescendant={flat()[active()] ? rowId(flat()[active()]) : undefined}
            placeholder="Search sandboxes, nodes, models, docs…"
            value={q()}
            onInput={(event) => setQ(event.currentTarget.value)}
            autocomplete="off"
            spellcheck={false}
          />
          <div class="palette-chips">
            <label class="palette-chip" classList={{ on: kind() !== '' }}>
              <span>Kind</span>
              <select value={kind()} onChange={(event) => setKind(event.currentTarget.value)} aria-label="Filter by kind">
                <option value="">All</option>
                <For each={KINDS}>{(value) => <option value={value}>{KIND_LABELS[value]}</option>}</For>
              </select>
            </label>
            <label class="palette-chip" classList={{ on: environment() !== '' }}>
              <span>Data center</span>
              <select value={environment()} onChange={(event) => setEnvironment(event.currentTarget.value)} aria-label="Filter by data center">
                <option value="">All</option>
                <For each={props.environments || []}>{(env) => <option value={env.id}>{env.name}</option>}</For>
              </select>
            </label>
            <label class="palette-chip" classList={{ on: status() !== '' }}>
              <span>Status</span>
              <select value={status()} onChange={(event) => setStatus(event.currentTarget.value)} aria-label="Filter by status">
                <option value="">All</option>
                <For each={STATUSES}>{(value) => <option value={value}>{value}</option>}</For>
              </select>
            </label>
            <Show when={loading()}>
              <span class="muted small palette-loading">Loading…</span>
            </Show>
          </div>

          <div class="palette-results" id="palette-list" role="listbox" aria-label="Results">
            <For each={groups()}>
              {(group) => (
                <div class="palette-group" role="group" aria-label={group.label}>
                  <div class="eyebrow palette-heading">{group.label}</div>
                  <For each={group.items}>
                    {(item) => {
                      const isActive = () => flat()[active()]?.id === item.id;
                      return (
                        <div
                          id={rowId(item)}
                          class="palette-row"
                          classList={{ active: isActive() }}
                          role="option"
                          aria-selected={isActive()}
                          data-row-id={item.id}
                          onMouseMove={() => setActive(flat().findIndex((row) => row.id === item.id))}
                          onClick={() => choose(item)}
                        >
                          <span class="palette-glyph mono" aria-hidden="true">{GLYPHS[item.kind] || '··'}</span>
                          <span class="palette-name">{item.name}</span>
                          <span class="palette-sub muted small">
                            {KIND_LABELS[item.kind] || item.kind}
                            {item.subtitle ? ` · ${item.subtitle}` : ''}
                          </span>
                          <Show when={item.status}>
                            <StatusBadge status={item.status} />
                          </Show>
                          <Show when={isActive()}>
                            <kbd class="palette-key">Enter</kbd>
                          </Show>
                        </div>
                      );
                    }}
                  </For>
                </div>
              )}
            </For>

            <Show when={failure()}>
              <p class="palette-note bad-text" role="alert">Server search failed: {failure()}</p>
            </Show>
            <Show when={!q().trim() && flat().length === 0}>
              <p class="palette-note muted">Type to search. Press Escape to close.</p>
            </Show>
            <Show when={q().trim() && flat().length === 0 && !loading() && !failure()}>
              <p class="palette-note muted">
                Nothing found. Try the docs:{' '}
                <a href="/docs" onClick={(event) => { event.preventDefault(); props.onOpen('/docs'); }}>
                  Open the documentation
                </a>
              </p>
            </Show>
          </div>
        </div>
      </div>
    </Show>
  );
}
