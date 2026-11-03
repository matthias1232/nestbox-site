import { For, Show, createMemo, createSignal } from 'solid-js';
import '../styles/primitives.css';
import Empty from './Empty.jsx';
import { sortRows, nextSort, toggle, rangeSelect } from './table.js';

const ROW = 32;
const WINDOW_OVER = 400;

// A selectable, sortable table (contract C15; the older DataTable stays for existing pages).
// Props: columns [{key,label,sortable,render,align,width,type}], rows, rowKey (key name or fn), selectable,
// selected (Set), onSelect(ids), sort {key,dir}, onSort(sort), onRowClick(row), empty {title,hint,action}.
// With no `sort`/`onSort` the table sorts itself; Shift+click on a row checkbox selects a range.
export default function SelectTable(props) {
  const [innerSort, setInnerSort] = createSignal(null);
  const [anchor, setAnchor] = createSignal(null);
  const [scrollTop, setScrollTop] = createSignal(0);
  const currentSort = () => (props.onSort ? props.sort : innerSort());
  const idOf = (row) => (typeof props.rowKey === 'function' ? props.rowKey(row) : row[props.rowKey]);
  const ordered = createMemo(() => {
    const s = currentSort();
    if (!s || props.onSort) return props.rows || [];
    const column = props.columns.find((c) => c.key === s.key);
    return sortRows(props.rows || [], s.key, s.dir, column && column.type);
  });
  const ids = createMemo(() => ordered().map(idOf));
  const selected = () => props.selected || new Set();
  const windowed = () => ordered().length > WINDOW_OVER;
  const first = () => (windowed() ? Math.max(0, Math.floor(scrollTop() / ROW) - 10) : 0);
  const last = () => (windowed() ? Math.min(ordered().length, first() + 40 + 20) : ordered().length);
  const visible = () => ordered().slice(first(), last());
  const allOn = () => ids().length > 0 && ids().every((id) => selected().has(id));

  const clickHeader = (column) => {
    if (!column.sortable) return;
    const next = nextSort(currentSort(), column.key);
    if (props.onSort) props.onSort(next);
    else setInnerSort(next);
  };
  const ariaSort = (column) => {
    const s = currentSort();
    if (!column.sortable) return undefined;
    return s && s.key === column.key ? (s.dir === 'asc' ? 'ascending' : 'descending') : 'none';
  };
  const emit = (set) => props.onSelect && props.onSelect(set);
  const pick = (id, event) => {
    if (event.shiftKey && anchor() !== null) {
      emit(new Set([...selected(), ...rangeSelect(ids(), anchor(), id)]));
    } else {
      emit(toggle(selected(), id));
    }
    setAnchor(id);
  };

  return (
    <Show when={(props.rows || []).length > 0} fallback={<Empty title={props.empty?.title || 'Nothing here'} hint={props.empty?.hint} action={props.empty?.action} />}>
      <div class="select-table-wrap" onScroll={(event) => windowed() && setScrollTop(event.currentTarget.scrollTop)} style={windowed() ? { 'max-height': '70vh' } : undefined}>
        <table class="select-table">
          <thead>
            <tr>
              <Show when={props.selectable}>
                <th class="check">
                  <input
                    type="checkbox"
                    aria-label="Select all rows"
                    checked={allOn()}
                    onChange={() => emit(allOn() ? new Set() : new Set(ids()))}
                  />
                </th>
              </Show>
              <For each={props.columns}>
                {(column) => (
                  <th
                    aria-sort={ariaSort(column)}
                    classList={{ num: column.align === 'right' }}
                    style={{ width: column.width, 'text-align': column.align }}
                  >
                    <Show when={column.sortable} fallback={column.label}>
                      <button type="button" class="ghost tiny sort" onClick={() => clickHeader(column)}>
                        {column.label}
                        <span aria-hidden="true">{currentSort()?.key === column.key ? (currentSort().dir === 'asc' ? ' ▲' : ' ▼') : ''}</span>
                      </button>
                    </Show>
                  </th>
                )}
              </For>
            </tr>
          </thead>
          <tbody>
            <Show when={windowed() && first() > 0}>
              <tr aria-hidden="true" style={{ height: `${first() * ROW}px` }} />
            </Show>
            <For each={visible()}>
              {(row) => {
                const id = idOf(row);
                return (
                  <tr
                    classList={{ selected: selected().has(id), clickable: !!props.onRowClick }}
                    aria-selected={props.selectable ? selected().has(id) : undefined}
                    onClick={() => props.onRowClick && props.onRowClick(row)}
                  >
                    <Show when={props.selectable}>
                      <td class="check" onClick={(event) => event.stopPropagation()}>
                        <input type="checkbox" aria-label="Select row" checked={selected().has(id)} onClick={(event) => pick(id, event)} />
                      </td>
                    </Show>
                    <For each={props.columns}>
                      {(column) => (
                        <td classList={{ num: column.align === 'right' }} style={{ 'text-align': column.align }}>
                          {column.render ? column.render(row) : row[column.key]}
                        </td>
                      )}
                    </For>
                  </tr>
                );
              }}
            </For>
            <Show when={windowed() && last() < ordered().length}>
              <tr aria-hidden="true" style={{ height: `${(ordered().length - last()) * ROW}px` }} />
            </Show>
          </tbody>
        </table>
      </div>
    </Show>
  );
}
