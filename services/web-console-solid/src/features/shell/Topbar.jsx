// The top bar: menu button, wordmark, the data-center picker, search, alerts, help and the user.
import { Show, createSignal, onCleanup } from 'solid-js';
import { useSession } from '../../state/session.jsx';
import { useUi } from '../../state/ui.jsx';
import ContextBar from '../tenants/ContextBar.jsx';
import { navigate, paths } from '../../routing.js';

export default function Topbar(props) {
  const session = useSession();
  const ui = useUi();
  const [menuOpen, setMenuOpen] = createSignal(false);
  let userRoot;

  // Close the user menu on a click elsewhere or on Escape; both listeners go with the component.
  function onDocClick(event) {
    if (menuOpen() && userRoot && !userRoot.contains(event.target)) setMenuOpen(false);
  }
  function onDocKey(event) {
    if (event.key === 'Escape') setMenuOpen(false);
  }
  document.addEventListener('click', onDocClick);
  document.addEventListener('keydown', onDocKey);
  onCleanup(() => {
    document.removeEventListener('click', onDocClick);
    document.removeEventListener('keydown', onDocKey);
  });

  const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || '');

  return (
    <header class="cc-topbar">
      <button class="ghost cc-menu-button" aria-label="Open the menu" onClick={() => props.onMenu?.()}>
        <span aria-hidden="true">☰</span>
      </button>
      <a class="brand-mark" href="/" onClick={(event) => { event.preventDefault(); navigate(paths.home()); }}>nestbox<i>_</i></a>
      <div class="cc-context" data-tour="context"><ContextBar /></div>
      <button class="cc-search" data-tour="search" aria-label="Search" onClick={() => ui.palette.open()}>
        <span>Search…</span>
        <kbd class="cc-kbd mono">{isMac ? '⌘K' : 'Ctrl K'}</kbd>
      </button>
      <button
        class="ghost cc-bell"
        aria-label={`Alerts, ${ui.alerts.count()} waiting`}
        onClick={() => { ui.dock.setTab('alerts'); ui.dock.setOpen(true); }}
      >
        <span aria-hidden="true">Alerts</span>
        <span class="cc-bell-count mono" classList={{ hot: ui.alerts.count() > 0 }}>{ui.alerts.count()}</span>
      </button>
      <button class="ghost" aria-label="Help" onClick={() => ui.help.open()}>?</button>
      <div class="cc-user-menu" ref={userRoot}>
        <button
          class="ghost cc-user"
          aria-haspopup="menu"
          aria-expanded={menuOpen()}
          onClick={() => setMenuOpen(!menuOpen())}
        >
          {session.identity()?.display_name} · <span class="muted">{session.identity()?.role}</span>
        </button>
        <Show when={menuOpen()}>
          <div class="cc-menu-pop" role="menu">
            <button role="menuitem" onClick={() => { setMenuOpen(false); session.signOut(); }}>Sign out</button>
          </div>
        </Show>
      </div>
    </header>
  );
}
