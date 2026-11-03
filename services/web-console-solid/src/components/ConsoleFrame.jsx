// One console for a sandbox, like Proxmox's: a split button picks Shell, Desktop or SSH and a
// toolbar carries copy, paste, font size, fullscreen, Ctrl+Alt+Del and pop-out.
//
// Props: sandbox, role?, control?, mode?, onMode?(mode), onPopOut?(mode, tab).
// The shared-tab strip (presence dots, +, close for me / for all) stays inside TerminalTab, which
// owns the tab state and the WebSocket; the frame owns the mode, the font and the toolbar.
import { Show, createMemo, createSignal, lazy } from 'solid-js';
import Segmented from './Segmented.jsx';
import IconButton from './IconButton.jsx';
import { useUi } from '../state/ui.jsx';
import { useSession } from '../state/session.jsx';
import { terminalPath, desktopPath } from '../routing.js';
const TerminalTab = lazy(() => import('../features/sandboxes/TerminalTab.jsx'));
const DesktopTab = lazy(() => import('../features/sandboxes/DesktopTab.jsx'));
import ConnectPanel from '../features/sandboxes/ConnectPanel.jsx';
import { FONT_DEFAULT, modes, nextFont, pickMode } from './consoleFrame.js';
import '../styles/console.css';

const LABELS = { shell: 'Shell', desktop: 'Desktop', ssh: 'SSH' };

// The console frame of one sandbox.
export default function ConsoleFrame(props) {
  const ui = useUi();
  const session = useSession();
  let frame;
  const [font, setFont] = createSignal(FONT_DEFAULT);
  const [viewOnly, setViewOnly] = createSignal(false);
  const [term, setTerm] = createSignal(null);
  const [desk, setDesk] = createSignal(null);
  const offered = createMemo(() => modes(props.sandbox, props.role || session.identity()?.role));
  // The caller's mode wins; otherwise the last choice for this sandbox; otherwise the first.
  const mode = () => pickMode(props.mode || ui.console.mode(props.sandbox.id), offered());

  function choose(next) {
    ui.console.setMode(props.sandbox.id, next);
    props.onMode?.(next);
  }

  async function paste() {
    let text = null;
    if (window.isSecureContext && navigator.clipboard?.readText) {
      try { text = await navigator.clipboard.readText(); } catch { text = null; }
    }
    // Plain HTTP has no clipboard read: ask for the text instead.
    if (text === null) text = window.prompt('Paste the text to send to the terminal:');
    if (text) term()?.paste(text);
  }

  function fullscreen() {
    if (mode() === 'desktop' && desk()) desk().fullscreen();
    else frame?.requestFullscreen?.();
  }

  function popOut() {
    if (props.onPopOut) return props.onPopOut(mode(), term()?.activeTab?.());
    const id = props.sandbox.id;
    if (mode() === 'desktop') {
      window.open(desktopPath(id), `nb-desk-${id}`, 'popup,width=1440,height=900');
    } else {
      const tab = term()?.activeTab?.();
      if (tab) window.open(terminalPath(id, tab), `nb-term-${tab}`, 'popup,width=1200,height=800');
    }
  }

  return (
    <section class="console-frame" ref={frame}>
      <div class="console-toolbar" role="toolbar" aria-label="Console">
        <Segmented
          options={offered().map((value) => ({ value, label: LABELS[value] }))}
          value={mode()}
          onChange={choose}
        />
        <Show when={mode() === 'desktop' && viewOnly()}>
          <span class="badge info"><i />view only</span>
        </Show>
        <span class="console-spacer" />
        <Show when={mode() === 'shell'}>
          <IconButton icon="Settings" label="Smaller font" onClick={() => setFont(nextFont(font(), -1))} />
          <span class="mono small" aria-live="polite">{font()} px</span>
          <IconButton icon="Settings" label="Larger font" onClick={() => setFont(nextFont(font(), 1))} />
          <IconButton icon="Copy" label="Copy selection" onClick={() => term()?.copy()} />
          <button type="button" class="tiny" onClick={paste}>Paste</button>
        </Show>
        <Show when={mode() === 'desktop'}>
          <label class="small">
            <input type="checkbox" checked={viewOnly()} onChange={(e) => setViewOnly(e.currentTarget.checked)} />
            {' '}view only
          </label>
          <button type="button" class="tiny" disabled={viewOnly()} onClick={() => desk()?.sendCtrlAltDel()}>
            Ctrl+Alt+Del
          </button>
        </Show>
        <Show when={mode() !== 'ssh'}>
          <button type="button" class="tiny" onClick={fullscreen}>Fullscreen</button>
          <IconButton icon="PopOut" label="Pop out" onClick={popOut} />
        </Show>
      </div>
      <div class="console-body">
        <Show when={mode() === 'shell'}>
          <TerminalTab sandbox={props.sandbox} showAll fontSize={font()} onReady={setTerm} />
        </Show>
        <Show when={mode() === 'desktop'}>
          <DesktopTab sandbox={props.sandbox} viewOnly={viewOnly()} onReady={setDesk} />
        </Show>
        <Show when={mode() === 'ssh'}>
          <ConnectPanel sandboxId={props.sandbox.id} />
        </Show>
      </div>
    </section>
  );
}
