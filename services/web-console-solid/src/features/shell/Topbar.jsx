// The top bar: menu button, wordmark, the data-center picker, search, alerts, help and the user.
//
// The bell is a split control (U05): the bell itself opens the dock's Alerts tab as before; the
// small ▾ opens the pop-up menu (on, mute for an hour, mute until turned on). Muted, the bell
// draws the crossed bell, and a new refusal makes the count flash once — the count never stops.
import { Show, createEffect, createSignal, onCleanup } from 'solid-js';
import { useSession } from '../../state/session.jsx';
import { useUi } from '../../state/ui.jsx';
import ContextBar from '../tenants/ContextBar.jsx';
import KeyHint from '../../components/KeyHint.jsx';
import { Icon } from '../../components/icons.jsx';
import { navigate, paths } from '../../routing.js';

const HOUR_MS = 60 * 60 * 1000;

export default function Topbar(props) {
  const session = useSession();
  const ui = useUi();
  const [menuOpen, setMenuOpen] = createSignal(false);
  const [bellMenuOpen, setBellMenuOpen] = createSignal(false);
  const [flash, setFlash] = createSignal(false);
  let userRoot;
  let bellRoot;
  let flashTimer = null;
  let lastCount = ui.alerts.count();

  // Close a menu on a click elsewhere or on Escape; both listeners go with the component.
  function onDocClick(event) {
    if (menuOpen() && userRoot && !userRoot.contains(event.target)) setMenuOpen(false);
    if (bellMenuOpen() && bellRoot && !bellRoot.contains(event.target)) setBellMenuOpen(false);
  }
  function onDocKey(event) {
    if (event.key === 'Escape') {
      setMenuOpen(false);
      setBellMenuOpen(false);
    }
  }
  document.addEventListener('click', onDocClick);
  document.addEventListener('keydown', onDocKey);
  onCleanup(() => {
    document.removeEventListener('click', onDocClick);
    document.removeEventListener('keydown', onDocKey);
    if (flashTimer !== null) clearTimeout(flashTimer);
  });

  // A new refusal while the pop-ups are muted: flash the count once for 600 ms, nothing else.
  createEffect(() => {
    const count = ui.alerts.count();
    const muted = ui.alerts.popups() === 'off';
    if (count > lastCount && muted) {
      if (flashTimer !== null) clearTimeout(flashTimer);
      setFlash(false);
      setTimeout(() => setFlash(true), 0);
      flashTimer = setTimeout(() => {
        setFlash(false);
        flashTimer = null;
      }, 620);
    }
    lastCount = count;
  });

  function choosePopups(value) {
    setBellMenuOpen(false);
    ui.alerts.setPopups(value);
  }

  return (
    <header class="cc-topbar">
      <button class="ghost cc-menu-button" aria-label="Open the menu" onClick={() => props.onMenu?.()}>
        <span aria-hidden="true">☰</span>
      </button>
      <a class="brand-mark" href="/" onClick={(event) => { event.preventDefault(); navigate(paths.home()); }}>nestbox<i>_</i></a>
      <div class="cc-context" data-tour="context"><ContextBar /></div>
      <button class="cc-search" data-tour="search" aria-label="Search" onClick={() => ui.palette.open()}>
        <span>Search…</span>
        <KeyHint keys="mod+k" />
      </button>
      <div class="cc-bell-group" ref={bellRoot}>
        <button
          class="ghost cc-bell"
          aria-label={`Alerts, ${ui.alerts.count()} waiting`}
          onClick={() => { ui.dock.setTab('alerts'); ui.dock.setOpen(true); }}
        >
          <Show when={ui.alerts.popups() === 'off'} fallback={<Icon name="Bell" />}>
            <Icon name="BellOff" />
          </Show>
          <span
            class="cc-bell-count mono"
            classList={{ hot: ui.alerts.count() > 0, flash: flash() }}
          >{ui.alerts.count()}</span>
        </button>
        <button
          class="ghost cc-bell-more"
          aria-label="Alert pop-up settings"
          aria-haspopup="menu"
          aria-expanded={bellMenuOpen()}
          onClick={() => setBellMenuOpen(!bellMenuOpen())}
        >
          <span aria-hidden="true">▾</span>
        </button>
        <Show when={bellMenuOpen()}>
          <div class="cc-menu-pop cc-bell-menu" role="menu" aria-label="Alert pop-ups">
            <button role="menuitem" onClick={() => choosePopups('on')}>Pop-ups on</button>
            <button role="menuitem" onClick={() => choosePopups({ until: Date.now() + HOUR_MS })}>Mute pop-ups for 1 hour</button>
            <button role="menuitem" onClick={() => choosePopups('off')}>Mute pop-ups until I turn them on</button>
          </div>
        </Show>
      </div>
      <button class="ghost cc-icon-button" aria-label="Help" title="Help" onClick={() => ui.help.open()}><Icon name="Help" /></button>
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
