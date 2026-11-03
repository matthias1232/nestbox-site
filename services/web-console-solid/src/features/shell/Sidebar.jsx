// The sidebar: the inventory tree (package F01) above the menu groups.
// Skeleton from Wave 0: the menu groups only; `data-tour="tree"` marks where the tree goes.
import { For } from 'solid-js';
import { useSession } from '../../state/session.jsx';
import { useUi } from '../../state/ui.jsx';
import { menuFor } from '../../navigation.js';
import { navigate, paths } from '../../routing.js';

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

export default function Sidebar(props) {
  const session = useSession();
  const ui = useUi();

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

  return (
    <aside class="cc-sidebar" classList={{ collapsed: props.collapsed }}>
      <div class="cc-tree" data-tour="tree" />
      <nav class="cc-nav" aria-label="Menu">
        <For each={menuFor(session.identity()?.role)}>
          {(group) => (
            <section class="cc-nav-group">
              <h2 class="eyebrow">{group.label}</h2>
              <For each={group.routes}>
                {(item) => (
                  <button
                    aria-current={activeId(props.place) === item.id ? 'page' : undefined}
                    onClick={() => open(item)}
                  >
                    {item.label}
                  </button>
                )}
              </For>
            </section>
          )}
        </For>
      </nav>
    </aside>
  );
}
