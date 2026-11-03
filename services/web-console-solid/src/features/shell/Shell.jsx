// The frame around every page: topbar, sidebar, content and dock (contract C2).
// Skeleton from Wave 0; package F01 adds the overlay sidebar under 1100 px and the resizer.
import { For, createSignal } from 'solid-js';
import { useUi } from '../../state/ui.jsx';
import Topbar from './Topbar.jsx';
import Sidebar from './Sidebar.jsx';
import Dock from './Dock.jsx';
import { EXTRAS } from './pages.js';

export default function Shell(props) {
  const ui = useUi();
  const [overlay, setOverlay] = createSignal(false);
  return (
    <div class="cc-shell" classList={{ 'cc-overlay-open': overlay() }} style={{ '--sidebar-width': `${ui.sidebar.collapsed() ? 56 : ui.sidebar.width()}px` }}>
      <Topbar onMenu={() => setOverlay(!overlay())} />
      <Sidebar collapsed={ui.sidebar.collapsed()} place={props.place} />
      <main class="cc-content">{props.children}</main>
      <Dock />
      <For each={EXTRAS}>{(Extra) => <Extra />}</For>
    </div>
  );
}
