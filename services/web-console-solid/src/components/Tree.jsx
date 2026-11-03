import { For, Show, createMemo, createSignal } from 'solid-js';
import '../styles/primitives.css';
import { IconChevronRight, IconChevronDown } from './icons.jsx';
import { visibleRows, filterTree, nextIndex, toggleAction, idsWithChildren } from './tree.js';

const ROW = 32;
const WINDOW_OVER = 400;

// An expandable inventory tree with keyboard control (arrows move, left/right fold, Enter opens, Space selects).
// Props: root, selectedId, onOpen(node), onSelect(node, multi), expanded (Set), onToggle(id), filter,
// renderBadge(node), multiSelect.
export default function Tree(props) {
  const [cursor, setCursor] = createSignal(0);
  const [scrollTop, setScrollTop] = createSignal(0);
  const [viewport, setViewport] = createSignal(600);
  const shown = createMemo(() => filterTree(props.root, props.filter));
  // While a filter is active every ancestor of a match is shown open.
  const openSet = createMemo(() => {
    if (!String(props.filter || '').trim()) return props.expanded || new Set();
    return new Set([...(props.expanded || []), ...idsWithChildren(shown())]);
  });
  const rows = createMemo(() => visibleRows(shown(), openSet()));
  const windowed = () => rows().length > WINDOW_OVER;
  const first = () => (windowed() ? Math.max(0, Math.floor(scrollTop() / ROW) - 10) : 0);
  const last = () => (windowed() ? Math.min(rows().length, first() + Math.ceil(viewport() / ROW) + 20) : rows().length);
  const slice = () => rows().slice(first(), last());
  let box;

  const focusRow = (index) => {
    setCursor(index);
    if (windowed() && box) {
      const top = index * ROW;
      if (top < box.scrollTop) box.scrollTop = top;
      else if (top + ROW > box.scrollTop + box.clientHeight) box.scrollTop = top + ROW - box.clientHeight;
    }
    queueMicrotask(() => box && box.querySelector(`[data-index="${index}"]`)?.focus());
  };

  const onKeyDown = (event) => {
    const index = Math.min(cursor(), rows().length - 1);
    const row = rows()[index];
    if (!row) return;
    const key = event.key;
    if (['ArrowDown', 'ArrowUp', 'Home', 'End', 'ArrowLeft', 'ArrowRight'].includes(key)) {
      event.preventDefault();
      const action = toggleAction(rows(), index, key, openSet());
      if (action) props.onToggle(row.node.id);
      else focusRow(nextIndex(rows(), index, key, openSet()));
    } else if (key === 'Enter') {
      event.preventDefault();
      props.onOpen && props.onOpen(row.node);
    } else if (key === ' ') {
      event.preventDefault();
      props.onSelect && props.onSelect(row.node, !!props.multiSelect);
    }
  };

  return (
    <div
      class="tree"
      role="tree"
      aria-multiselectable={props.multiSelect ? 'true' : undefined}
      ref={(el) => {
        box = el;
        queueMicrotask(() => setViewport(el.clientHeight || 600));
      }}
      onScroll={(event) => windowed() && setScrollTop(event.currentTarget.scrollTop)}
      onKeyDown={onKeyDown}
    >
      <Show when={shown()} fallback={<p class="muted small tree-empty">Nothing matches.</p>}>
        <div style={windowed() ? { height: `${rows().length * ROW}px`, position: 'relative' } : undefined}>
          <div style={windowed() ? { transform: `translateY(${first() * ROW}px)` } : undefined}>
            <For each={slice()}>
              {(row, i) => {
                const index = () => first() + i();
                return (
                  <div
                    class="tree-row"
                    role="treeitem"
                    data-index={index()}
                    data-kind={row.node.kind}
                    aria-level={row.depth + 1}
                    aria-expanded={row.hasChildren ? openSet().has(row.node.id) : undefined}
                    aria-selected={props.selectedId === row.node.id}
                    classList={{ selected: props.selectedId === row.node.id, cursor: cursor() === index() }}
                    tabindex={cursor() === index() ? 0 : -1}
                    style={{ 'padding-left': `${row.depth * 16 + 4}px` }}
                    onFocus={() => setCursor(index())}
                    onClick={(event) => {
                      setCursor(index());
                      if (props.multiSelect && (event.shiftKey || event.ctrlKey || event.metaKey)) {
                        props.onSelect && props.onSelect(row.node, true);
                      } else {
                        props.onSelect && props.onSelect(row.node, false);
                        props.onOpen && props.onOpen(row.node);
                      }
                    }}
                  >
                    <span
                      class="tree-twisty"
                      aria-hidden="true"
                      onClick={(event) => {
                        event.stopPropagation();
                        if (row.hasChildren) props.onToggle(row.node.id);
                      }}
                    >
                      <Show when={row.hasChildren}>
                        {openSet().has(row.node.id) ? <IconChevronDown size={12} /> : <IconChevronRight size={12} />}
                      </Show>
                    </span>
                    {/* The organisation is a section label like the mockup's PLATFORM: eyebrow
                        face, no status dot. Everything below it is an object with a state. */}
                    <Show when={row.node.kind !== 'organization'}>
                      <i class="status-dot" data-status={row.node.status || 'unknown'} title={row.node.status || ''} />
                    </Show>
                    <span class="tree-name" classList={{ eyebrow: row.node.kind === 'organization' }}>{row.node.name}</span>
                    <Show when={row.node.status}>
                      <span class="visually-hidden"> ({row.node.status})</span>
                    </Show>
                    {props.renderBadge && props.renderBadge(row.node)}
                  </div>
                );
              }}
            </For>
          </div>
        </div>
      </Show>
    </div>
  );
}
