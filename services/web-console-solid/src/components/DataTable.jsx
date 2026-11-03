// One table for every list in the console: sort, filter, search, group, export.
//
// The work lives in `dataTable.js`; this file is the interaction. Live mode adds
// new rows at the top with a short highlight, and holds them back when the reader
// has scrolled away from the top, so a running list does not fight the reader.
import { For, Show, createEffect, createMemo, createSignal, onCleanup } from 'solid-js';
import { cellText, filterRows, groupRows, searchRows, sortRows, toCsv } from './dataTable.js';
import '../styles/dlp.css';

const PAGE = 500;
const NEW_ROW_MS = 600;

function readStored(key) {
  if (!key || typeof localStorage === 'undefined') return {};
  try {
    const raw = localStorage.getItem(key);
    const parsed = raw ? JSON.parse(raw) : null;
    return parsed && typeof parsed === 'object' ? parsed : {};
  } catch {
    return {};
  }
}

function writeStored(key, value) {
  if (!key || typeof localStorage === 'undefined') return;
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // A full or blocked storage must not break the table.
  }
}

/**
 * The shared table.
 *
 * Props: `rows`, `columns` (`{key, label, value?, render?, sortable?,
 * groupable?, filter?: 'text'|'select', class?}`), `rowKey(row)`, `defaultSort`,
 * `defaultGroup`, `storageKey` to remember sort/group/filters, `live` to
 * highlight new rows, `toolbar`, `onRowClick(row)`, `rowClass(row)` and
 * `emptyText`.
 */
export default function DataTable(props) {
  const stored = readStored(props.storageKey);
  const columns = () => props.columns || [];

  const [sort, setSort] = createSignal(
    stored.sort !== undefined
      ? stored.sort && typeof stored.sort === 'object'
        ? stored.sort
        : null
      : props.defaultSort || null,
  );
  const [groupKey, setGroupKey] = createSignal(
    stored.group !== undefined ? stored.group : props.defaultGroup || null,
  );
  const [filters, setFilters] = createSignal(
    stored.filters && typeof stored.filters === 'object' ? stored.filters : {},
  );
  const [search, setSearch] = createSignal('');
  const [shown, setShown] = createSignal(PAGE);
  const [collapsed, setCollapsed] = createSignal({});
  const [newKeys, setNewKeys] = createSignal(new Set());
  const [frozen, setFrozen] = createSignal(null);
  const [pending, setPending] = createSignal(0);

  let scroller;
  let atTop = true;
  let frozenNow = false;
  let held = [];
  let highlightTimer = null;

  function keyOf(row) {
    return props.rowKey ? props.rowKey(row) : row?.id;
  }

  let known = new Set((props.rows || []).map(keyOf));
  let lastRows = props.rows || [];

  createEffect(() => {
    if (!props.storageKey) return;
    writeStored(props.storageKey, { sort: sort(), group: groupKey(), filters: filters() });
  });

  const filtered = createMemo(() => {
    let current = props.rows || [];
    current = searchRows(current, columns(), search());
    current = filterRows(current, columns(), filters());
    return sortRows(current, columns(), sort());
  });

  // While new rows are held back, the reader keeps seeing the previous snapshot,
  // still sorted and filtered the way the toolbar currently says.
  const displayRows = createMemo(() => {
    const base = frozen();
    if (!base) return filtered();
    let current = searchRows(base, columns(), search());
    current = filterRows(current, columns(), filters());
    return sortRows(current, columns(), sort());
  });

  const grouped = createMemo(() => {
    const column = columns().find((candidate) => candidate.key === groupKey());
    return groupRows(displayRows().slice(0, shown()), column);
  });

  function markNew(keys) {
    if (!keys.length) return;
    setNewKeys(new Set(keys));
    if (highlightTimer) clearTimeout(highlightTimer);
    highlightTimer = setTimeout(() => {
      setNewKeys(new Set());
      highlightTimer = null;
    }, NEW_ROW_MS);
  }

  function unfreeze() {
    frozenNow = false;
    setFrozen(null);
  }

  function dropHeld() {
    held = [];
    lastRows = props.rows || [];
    unfreeze();
    setPending(0);
  }

  function releaseHeld() {
    const keys = held.map(keyOf);
    held = [];
    lastRows = props.rows || [];
    unfreeze();
    setPending(0);
    if (keys.length) markNew(keys);
    atTop = true;
    if (scroller) scroller.scrollTop = 0;
  }

  // Watch the row keys, not the rows: a re-render of the same data is not news.
  createEffect(() => {
    const incoming = props.rows || [];
    const keys = incoming.map(keyOf);
    const fresh = keys.filter((key) => !known.has(key));
    known = new Set(keys);

    if (!props.live) {
      held = [];
      if (frozenNow) unfreeze();
      setPending(0);
      lastRows = incoming;
      return;
    }
    if (!fresh.length) {
      if (!frozenNow) lastRows = incoming;
      return;
    }
    if (atTop) {
      held = [];
      lastRows = incoming;
      unfreeze();
      setPending(0);
      markNew(fresh);
      return;
    }
    const byKey = new Map(incoming.map((row) => [keyOf(row), row]));
    for (const key of fresh) {
      const row = byKey.get(key);
      if (row !== undefined) held.push(row);
    }
    frozenNow = true;
    setFrozen(lastRows);
    setPending(held.length);
  });

  onCleanup(() => {
    if (highlightTimer) clearTimeout(highlightTimer);
  });

  function toggleSort(column) {
    if (!column.sortable) return;
    dropHeld();
    const current = sort();
    if (!current || current.key !== column.key) setSort({ key: column.key, dir: 'asc' });
    else if (current.dir === 'asc') setSort({ key: column.key, dir: 'desc' });
    else setSort(null);
  }

  function toggleGroup(key) {
    setCollapsed({ ...collapsed(), [key]: !collapsed()[key] });
  }

  function options(column) {
    const values = new Set();
    for (const row of props.rows || []) {
      const value = cellText(row, column);
      if (value !== '') values.add(value);
    }
    return [...values].sort((left, right) =>
      left.localeCompare(right, undefined, { numeric: true, sensitivity: 'base' }),
    );
  }

  function setFilter(key, value) {
    dropHeld();
    setFilters({ ...filters(), [key]: value });
    setShown(PAGE);
  }

  function toggleFilterValue(key, value) {
    const current = filters()[key];
    const list = Array.isArray(current) ? current : [];
    const next = list.includes(value) ? list.filter((entry) => entry !== value) : [...list, value];
    setFilter(key, next);
  }

  function changeSearch(value) {
    dropHeld();
    setSearch(value);
    setShown(PAGE);
  }

  function changeGroup(value) {
    dropHeld();
    setGroupKey(value || null);
    setShown(PAGE);
  }

  function download() {
    const csv = toCsv(filtered(), columns());
    const blob = new Blob([csv], { type: 'text/csv;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = `${props.storageKey || 'table'}.csv`;
    link.click();
    URL.revokeObjectURL(url);
  }

  function handleScroll() {
    atTop = (scroller?.scrollTop || 0) <= 8;
    if (atTop && pending() > 0) releaseHeld();
  }

  const rowClass = (row) => {
    const extra = props.rowClass ? props.rowClass(row) : '';
    const fresh = newKeys().has(keyOf(row)) ? 'row-new' : '';
    return `${extra || ''} ${fresh}`.trim();
  };

  return (
    <section class="data-table">
      <div class="controls data-table-toolbar">
        <input
          class="input"
          type="search"
          placeholder="Search…"
          aria-label="Search the table"
          value={search()}
          onInput={(event) => changeSearch(event.currentTarget.value)}
        />
        <label class="data-table-group">
          <span class="muted small">Group by</span>
          <select
            value={groupKey() || ''}
            onChange={(event) => changeGroup(event.currentTarget.value)}
          >
            <option value="">none</option>
            <For each={columns().filter((column) => column.groupable)}>
              {(column) => <option value={column.key}>{column.label}</option>}
            </For>
          </select>
        </label>
        <button class="secondary tiny" onClick={download} title="All filtered rows as CSV">
          CSV
        </button>
        {props.toolbar}
      </div>

      <Show when={pending() > 0}>
        <button class="new-rows" onClick={releaseHeld}>
          {pending()} new
        </button>
      </Show>

      <div class="table-scroll data-table-scroll" ref={scroller} onScroll={handleScroll}>
        <table class="dense">
          <thead>
            <tr>
              <For each={columns()}>
                {(column) => (
                  <th
                    scope="col"
                    class={column.sortable ? 'sortable' : ''}
                    aria-sort={
                      sort()?.key === column.key
                        ? sort().dir === 'asc'
                          ? 'ascending'
                          : 'descending'
                        : 'none'
                    }
                    onClick={() => toggleSort(column)}
                  >
                    {column.label}
                    <Show when={sort()?.key === column.key}>
                      <span class="sort-arrow" aria-hidden="true">
                        {sort().dir === 'asc' ? ' ▲' : ' ▼'}
                      </span>
                    </Show>
                  </th>
                )}
              </For>
            </tr>
            <Show when={columns().some((column) => column.filter)}>
              <tr class="filter-row">
                <For each={columns()}>
                  {(column) => (
                    <th>
                      <Show when={column.filter === 'text'}>
                        <input
                          class="input tiny"
                          aria-label={`${column.label} filter`}
                          value={
                            typeof filters()[column.key] === 'string' ? filters()[column.key] : ''
                          }
                          onInput={(event) => setFilter(column.key, event.currentTarget.value)}
                        />
                      </Show>
                      <Show when={column.filter === 'select'}>
                        <details class="filter-select">
                          <summary class="small">
                            {Array.isArray(filters()[column.key]) && filters()[column.key].length
                              ? `${filters()[column.key].length} selected`
                              : 'all'}
                          </summary>
                          <div class="filter-menu">
                            <For each={options(column)}>
                              {(value) => (
                                <label class="filter-option">
                                  <input
                                    type="checkbox"
                                    checked={
                                      Array.isArray(filters()[column.key]) &&
                                      filters()[column.key].includes(value)
                                    }
                                    onChange={() => toggleFilterValue(column.key, value)}
                                  />
                                  <span>{value}</span>
                                </label>
                              )}
                            </For>
                            <Show
                              when={
                                Array.isArray(filters()[column.key]) && filters()[column.key].length
                              }
                            >
                              <button
                                class="secondary tiny"
                                onClick={() => setFilter(column.key, [])}
                              >
                                reset
                              </button>
                            </Show>
                          </div>
                        </details>
                      </Show>
                    </th>
                  )}
                </For>
              </tr>
            </Show>
          </thead>
          <tbody>
            <Show
              when={displayRows().length}
              fallback={
                <tr>
                  <td class="muted small" colSpan={columns().length || 1}>
                    {props.emptyText || 'No rows.'}
                  </td>
                </tr>
              }
            >
              <For each={grouped()}>
                {(group) => (
                  <>
                    <Show when={group.key}>
                      <tr class="group-row">
                        <td colSpan={columns().length || 1}>
                          <button
                            class="group-toggle"
                            aria-expanded={!collapsed()[group.key]}
                            onClick={() => toggleGroup(group.key)}
                          >
                            <span aria-hidden="true">{collapsed()[group.key] ? '▸' : '▾'}</span>
                            {group.label}
                            <span class="muted"> ({group.count})</span>
                          </button>
                        </td>
                      </tr>
                    </Show>
                    <Show when={!collapsed()[group.key]}>
                      <For each={group.rows}>
                        {(row) => (
                          <tr
                            class={`${rowClass(row)} ${props.onRowClick ? 'clickable' : ''}`.trim()}
                            onClick={() => props.onRowClick?.(row)}
                          >
                            <For each={columns()}>
                              {(column) => (
                                <td class={column.class || ''}>
                                  {column.render ? column.render(row) : cellText(row, column)}
                                </td>
                              )}
                            </For>
                          </tr>
                        )}
                      </For>
                    </Show>
                  </>
                )}
              </For>
            </Show>
          </tbody>
        </table>
      </div>

      <Show when={displayRows().length > shown()}>
        <div class="table-more">
          <button class="secondary tiny" onClick={() => setShown((count) => count + PAGE)}>
            Show {PAGE} more
          </button>
          <span class="muted small">
            {shown()} of {displayRows().length}
          </span>
        </div>
      </Show>
    </section>
  );
}
