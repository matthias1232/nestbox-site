// The top bar: menu button, wordmark, the data-center picker, search, alerts, help and the user.
// Skeleton from Wave 0; package F01 gives it the real search trigger, bell and user menu.
import { useSession } from '../../state/session.jsx';
import { useUi } from '../../state/ui.jsx';
import ContextBar from '../tenants/ContextBar.jsx';
import { navigate, paths } from '../../routing.js';

export default function Topbar(props) {
  const session = useSession();
  const ui = useUi();
  return (
    <header class="cc-topbar">
      <button class="ghost cc-menu-button" aria-label="Open the menu" onClick={() => props.onMenu?.()}>Menu</button>
      <a class="brand-mark" href="/" onClick={(event) => { event.preventDefault(); navigate(paths.home()); }}>nestbox<i>_</i></a>
      <div class="cc-context" data-tour="context"><ContextBar /></div>
      <button class="secondary cc-search" data-tour="search" onClick={() => ui.palette.open()}>
        Search <span class="muted small mono">Ctrl K</span>
      </button>
      <button class="ghost" aria-label="Alerts" onClick={() => { ui.dock.setTab('alerts'); ui.dock.setOpen(true); }}>
        Alerts <span class="mono">{ui.alerts.count()}</span>
      </button>
      <button class="ghost" aria-label="Help" onClick={() => ui.help.open()}>Help</button>
      <span class="muted small cc-user">{session.identity()?.display_name} · {session.identity()?.role}</span>
      <button class="secondary" onClick={session.signOut}>Sign out</button>
    </header>
  );
}
