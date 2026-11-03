// The frame around every page: topbar, sidebar, content and dock (contract C2).
// Under 1100 px the sidebar is an overlay with a backdrop; it closes on navigate and Escape.
import { For, Show, createSignal, onCleanup } from 'solid-js';
import { useUi, onKey } from '../../state/ui.jsx';
import Topbar from './Topbar.jsx';
import Sidebar from './Sidebar.jsx';
import Dock from './Dock.jsx';
import { EXTRAS } from './pages.js';
import '../../styles/shell.css';

export default function Shell(props) {
  const ui = useUi();
  const [overlay, setOverlay] = createSignal(false);

  const close = () => setOverlay(false);
  window.addEventListener('popstate', close);
  const stopEscape = onKey('Escape', close);
  onCleanup(() => {
    window.removeEventListener('popstate', close);
    stopEscape();
  });

  return (
    <div class="cc-shell" classList={{ 'cc-overlay-open': overlay() }} style={{ '--sidebar-width': `${ui.sidebar.collapsed() ? 56 : ui.sidebar.width()}px` }}>
      <Topbar onMenu={() => setOverlay(!overlay())} />
      <Sidebar collapsed={ui.sidebar.collapsed() && !overlay()} place={props.place} />
      <Show when={overlay()}>
        <div class="cc-backdrop" onClick={close} aria-hidden="true" />
      </Show>
      <main class="cc-content">{props.children}</main>
      <Dock />
      <For each={EXTRAS}>{(Extra) => <Extra />}</For>
    </div>
  );
}
