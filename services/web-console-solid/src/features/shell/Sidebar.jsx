// The sidebar: the inventory tree (three views, filter box) above the collapsible menu groups.
// Collapsed (56 px) it shows menu icons only; the right edge is a drag handle for the width.
import { For, Show, createMemo, createSignal, onCleanup } from 'solid-js';
import { useSession } from '../../state/session.jsx';
import { useUi } from '../../state/ui.jsx';
import { menuFor } from '../../navigation.js';
import { navigate, paths } from '../../routing.js';
import Segmented from '../../components/Segmented.jsx';
import Tree from '../../components/Tree.jsx';
import { useInventory } from './useInventory.js';
import { ancestorIds, expandableIds, filterTree, pathFor } from './treeFilter.js';

const VIEWS = [
  { value: 'infrastructure', label: 'Infra' },
  { value: 'workloads', label: 'Work' },
  { value: 'models', label: 'Models' },
];
const MIN_WIDTH = 220;
const MAX_WIDTH = 420;

// The menu id a place belongs to, so its entry is highlighted.
function activeId(place) {
  if (!place) return 'home';
  if (place.kind === 'menu') return place.id;
  if (place.kind === 'home') return 'home';
  if (place.kind === 'my' || place.kind === 'box') return 'my';
  if (place.kind === 'admin') return 'sandboxes';
  if (place.kind === 'model') return 'models';
  if (place.kind === 'cluster') return 'clusters';
  if (place.kind === 'docs') return 'docs';
  return null;
}

// The tree node a place points at (route kind to tree id), or null on a menu page.
function selectedOf(place) {
  if (!place || !place.id) return null;
  return ['dc', 'site', 'node', 'admin', 'box', 'model', 'cluster'].includes(place.kind) ? place.id : null;
}

// The first two letters of a menu label, the collapsed sidebar's stand-in for an icon.
function initials(label) {
  return label.replace(/[^A-Za-z]/g, '').slice(0, 2);
}

export default function Sidebar(props) {
  const session = useSession();
  const ui = useUi();
  const inv = useInventory();
  const [filterText, setFilterText] = createSignal('');
  const [appliedFilter, setAppliedFilter] = createSignal('');
  const [expanded, setExpanded] = createSignal(new Set());
  const [closedGroups, setClosedGroups] = createSignal(new Set());

  let filterTimer = null;
  onCleanup(() => filterTimer && clearTimeout(filterTimer));
  function onFilter(value) {
    setFilterText(value);
    if (filterTimer) clearTimeout(filterTimer);
    filterTimer = setTimeout(() => setAppliedFilter(value), 150);
  }

  const shown = createMemo(() => filterTree(inv.root(), appliedFilter()));
  // With a filter every branch is open so the matches are visible; otherwise the user's choice
  // plus the way to the current place and the root.
  const openSet = createMemo(() => {
    const tree = shown();
    if (!tree) return new Set();
    if (appliedFilter().trim()) return expandableIds(tree);
    const out = new Set(expanded());
    out.add(tree.id);
    const place = props.place;
    const kind = place?.kind;
    if (place?.id && kind) ancestorIds(tree, kind, place.id).forEach((id) => out.add(id));
    return out;
  });

  function toggle(id) {
    const next = new Set(expanded());
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setExpanded(next);
  }

  function open(item) {
    if (item.action === 'dock') {
      ui.dock.setTab(item.id);
      ui.dock.setOpen(true);
    } else if (item.action === 'shortcuts') {
      ui.shortcuts.open();
    } else {
      navigate(paths.menu(item.id));
    }
  }

  function toggleGroup(id) {
    const next = new Set(closedGroups());
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setClosedGroups(next);
  }

  // A badge per tree node: refused destinations in red with a word, allow-all as a lime dot.
  function renderBadge(node) {
    const alerts = typeof node.alerts === 'number' ? node.alerts : node.counts?.alerts || 0;
    const allowAll = node.allow_all === true || (node.counts?.allow_all || 0) > 0;
    return (
      <>
        <Show when={alerts > 0}>
          <span class="cc-badge-alert mono" title={`${alerts} alert${alerts === 1 ? '' : 's'}`}>{alerts} alert{alerts === 1 ? '' : 's'}</span>
        </Show>
        <Show when={allowAll}>
          <span class="cc-badge-open" title="Allow-all is on" role="img" aria-label="Allow-all is on" />
        </Show>
      </>
    );
  }

  // Drag the right edge to resize; the keyboard works too (arrow keys).
  let drag = null;
  const clamp = (value) => Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, Math.round(value)));
  function onDown(event) {
    drag = { x: event.clientX, width: ui.sidebar.width() };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }
  function onMove(event) {
    if (drag) ui.sidebar.setWidth(clamp(drag.width + event.clientX - drag.x));
  }
  function onUp() {
    drag = null;
  }
  function onHandleKey(event) {
    if (event.key === 'ArrowLeft') ui.sidebar.setWidth(clamp(ui.sidebar.width() - 16));
    else if (event.key === 'ArrowRight') ui.sidebar.setWidth(clamp(ui.sidebar.width() + 16));
    else return;
    event.preventDefault();
  }

  return (
    <aside class="cc-sidebar" classList={{ collapsed: props.collapsed }} aria-label="Navigation">
      <div class="cc-sidebar-head">
        <Show when={!props.collapsed}>
          <span class="eyebrow">Inventory</span>
        </Show>
        <button
          class="ghost tiny cc-collapse"
          aria-label={props.collapsed ? 'Expand the sidebar' : 'Collapse the sidebar'}
          aria-expanded={!props.collapsed}
          onClick={() => ui.sidebar.setCollapsed(!ui.sidebar.collapsed())}
        >
          {props.collapsed ? '»' : '«'}
        </button>
      </div>

      <Show when={!props.collapsed}>
        <div class="cc-tree-controls">
          <Segmented options={VIEWS} value={inv.view()} onChange={(value) => inv.setView(value)} />
          <input
            class="cc-tree-filter"
            type="search"
            placeholder="Filter the tree…"
            aria-label="Filter the tree"
            value={filterText()}
            onInput={(event) => onFilter(event.currentTarget.value)}
          />
        </div>
        <div class="cc-tree" data-tour="tree">
          <Show when={inv.error() && !inv.root()}>
            <p class="muted small cc-tree-note">Inventory unavailable: {inv.error()}</p>
          </Show>
          <Show when={!inv.root() && !inv.error()}>
            <p class="muted small cc-tree-note">Loading…</p>
          </Show>
          <Show when={shown()}>
            <Tree
              root={shown()}
              selectedId={selectedOf(props.place)}
              onOpen={(node) => navigate(pathFor(node))}
              onSelect={(node, multi) => {
                if (multi && node.kind === 'sandbox') ui.selection.toggle('sandbox', node.id);
              }}
              expanded={openSet()}
              onToggle={toggle}
              filter={appliedFilter()}
              renderBadge={renderBadge}
              multiSelect
            />
          </Show>
        </div>
      </Show>

      <nav class="cc-nav" aria-label="Menu">
        <For each={menuFor(session.identity()?.role)}>
          {(group) => (
            <section class="cc-nav-group">
              <Show when={!props.collapsed}>
                <h2 class="eyebrow">
                  <button
                    class="cc-group-toggle"
                    aria-expanded={!closedGroups().has(group.id)}
                    onClick={() => toggleGroup(group.id)}
                  >
                    {group.label}
                  </button>
                </h2>
              </Show>
              <Show when={props.collapsed || !closedGroups().has(group.id)}>
                <For each={group.routes}>
                  {(item) => (
                    <button
                      title={item.label}
                      aria-label={item.label}
                      aria-current={activeId(props.place) === item.id ? 'page' : undefined}
                      onClick={() => open(item)}
                    >
                      <Show when={props.collapsed} fallback={item.label}>
                        <span class="cc-nav-icon mono" aria-hidden="true">{initials(item.label)}</span>
                      </Show>
                    </button>
                  )}
                </For>
              </Show>
            </section>
          )}
        </For>
      </nav>

      <Show when={!props.collapsed}>
        <div
          class="cc-resizer"
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize the sidebar"
          aria-valuemin={MIN_WIDTH}
          aria-valuemax={MAX_WIDTH}
          aria-valuenow={ui.sidebar.width()}
          tabindex="0"
          onPointerDown={onDown}
          onPointerMove={onMove}
          onPointerUp={onUp}
          onPointerCancel={onUp}
          onKeyDown={onHandleKey}
        />
      </Show>
    </aside>
  );
}
