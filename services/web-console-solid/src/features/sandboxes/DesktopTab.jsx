// The sandbox's graphical desktop, in the browser.
//
// noVNC speaks the VNC protocol over the WebSocket the control API opens to the
// sandbox (through the runner). The desktop is the sandbox's own: XFCE and the
// same Chrome an agent drives through the browser MCP, so a person can watch,
// take over, and log in where an agent is asked to.
//
// Only a sandbox built with the desktop pack has one. For the others the panel
// says how to get one instead of showing a black rectangle.
//
// Watching without touching is enforced by the **server**: the stream is opened
// with `?view=1` and the control API drops keyboard, pointer, clipboard and
// resize messages before they reach the desktop. `rfb.viewOnly` alone is a
// promise from the client to itself, which anybody can take back in DevTools.
// The sidebar's "View only" sets both, so the switch is honest.
//
// Every noVNC option (scaling, quality, compression, dot cursor, shared mode,
// automatic reconnect) is stored per sandbox in `ui.console`, applied on connect
// and on change; the panel on the left is `components/VncSidebar.jsx`.
//
// Monitor tabs (U10b): the strip above the picture is `components/MonitorTabs.jsx`.
// `All` is the whole framebuffer with the person's scaling option; a monitor tab
// shows only that monitor, scaled to the box, with the clicks mapped back into
// framebuffer pixels by `monitorView.js` — one VNC connection either way, because
// the sandbox has one framebuffer and VNC sends all of it. The layout itself comes
// from `useDisplay.js` and is edited in the sidebar's Display section.
import { Show, createEffect, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import RFB from '@novnc/novnc/lib/rfb';
import * as deck from '../../api/deck.js';
import * as sandboxes from '../../api/sandboxes.js';
import { environmentUrl } from '../../api/client.js';
import { desktopPath } from '../../routing.js';
import { useUi } from '../../state/ui.jsx';
import VncSidebar, { VncStatus } from '../../components/VncSidebar.jsx';
import MonitorTabs from '../../components/MonitorTabs.jsx';
import { DEFAULTS, applyOptions, sanitizeOptions, sendCombo, withSticky } from './vncOptions.js';
import { useDisplay } from './useDisplay.js';
import { showAll, showMonitor } from './monitorView.js';
import { PRESETS, monitorSize } from './displayLayout.js';
import '../../styles/workbox-sessions.css';

export default function DesktopTab(props) {
  const ui = useUi();
  let screen;
  let rfb;
  let manualClose = false;
  let retryTimer = null;
  let sizeObserver = null;
  let cropObserver = null;
  let cropFrame = 0;
  const [state, setState] = createSignal('idle');
  const [reason, setReason] = createSignal(null);
  const [note, setNote] = createSignal(null);
  const [size, setSize] = createSignal(null);
  const [clipboard, setClipboard] = createSignal('');
  const [retryIn, setRetryIn] = createSignal(0);
  const [busy, setBusy] = createSignal(false);
  const [browser, setBrowser] = createSignal(null);
  // The live connection is a signal too, because the crop has to re-run when the
  // stream is replaced: a plain variable would leave the new client unconfigured.
  const [client, setClient] = createSignal(null);
  const running = () => props.sandbox.status === 'running';

  // The display layout: which monitors exist and where they sit (U10b/U10c).
  const display = useDisplay(() => props.sandbox.id);
  const monitors = () => display.layout()?.monitors || [];
  // The tab this window shows: a pop-out may pin one monitor, otherwise the stored
  // choice, and a remembered monitor that no longer exists falls back to `All`.
  const activeId = () => {
    if (props.fixedMonitor) {
      return monitors().some((monitor) => monitor.id === props.fixedMonitor) ? props.fixedMonitor : 'all';
    }
    const stored = ui.console.monitor(props.sandbox.id);
    return stored && stored !== 'all' && !monitors().some((monitor) => monitor.id === stored) ? 'all' : stored || 'all';
  };
  const activeMonitor = () => monitors().find((monitor) => monitor.id === activeId()) || null;
  // What the status line should call the picture: the monitor's own size when one
  // is cropped out, the framebuffer's when everything is shown.
  const shownSize = () => {
    const monitor = activeMonitor();
    return monitor ? { width: monitor.width, height: monitor.height } : size();
  };

  // The options of this sandbox: the browser's remembered taste, completed.
  const options = createMemo(() => sanitizeOptions(ui.console.vncOptions(props.sandbox.id) || DEFAULTS));
  const toast = (tone, text) => ui.toast.show({ tone, text, undo: null, ms: 5000 });

  async function loadBrowser() {
    try {
      const attached = await deck.mcpAttached(props.sandbox.id);
      setBrowser((attached.items || attached || []).some?.((item) => (item.server_key || item.key) === 'chrome-browser'));
    } catch {
      setBrowser(null);
    }
  }

  // The framebuffer size, read from noVNC's display (it changes on remote resize).
  function readSize() {
    const canvas = screen?.querySelector('canvas');
    const width = rfb?._display?.width || Number(canvas?.getAttribute('width')) || 0;
    const height = rfb?._display?.height || Number(canvas?.getAttribute('height')) || 0;
    setSize(width > 0 && height > 0 ? { width, height } : null);
    // The framebuffer arriving is also when a crop first has something to crop.
    if (activeMonitor()) recrop();
  }

  // The box a cropped monitor is fitted into: the picture area, not the canvas,
  // because the canvas is exactly what the crop resizes.
  function screenBox() {
    return { w: screen?.clientWidth || 0, h: screen?.clientHeight || 0 };
  }

  // Put the active monitor back a frame later. noVNC's own clip update runs on a
  // window resize and resizes the viewport to the whole box; re-cropping in the
  // same turn would be overwritten, in an animation frame it wins.
  function recrop() {
    const live = client();
    if (!live || !activeMonitor()) return;
    if (cropFrame) cancelAnimationFrame(cropFrame);
    cropFrame = requestAnimationFrame(() => {
      cropFrame = 0;
      if (live !== client()) return;
      const monitor = activeMonitor();
      if (monitor) showMonitor(live, monitor, screenBox());
    });
  }

  // The box changes when the window resizes, the sidebar collapses or the tab strip
  // grows, and a cropped monitor has to be refitted every time.
  function watchCrop(on) {
    if (on && !cropObserver && typeof ResizeObserver !== 'undefined') {
      cropObserver = new ResizeObserver(recrop);
      if (screen) cropObserver.observe(screen);
    } else if (!on && cropObserver) {
      cropObserver.disconnect();
      cropObserver = null;
    }
  }

  function cancelReconnect() {
    if (retryTimer !== null) {
      clearTimeout(retryTimer);
      retryTimer = null;
    }
    setRetryIn(0);
  }

  // Automatic reconnect: wait the configured delay, counting down in the status
  // line, then open the stream again. Cancelled when the tab goes away.
  function scheduleReconnect() {
    cancelReconnect();
    if (!options().reconnect || !running()) return;
    let left = options().reconnectDelay;
    setRetryIn(left);
    const tick = () => {
      left -= 1;
      if (left <= 0) {
        retryTimer = null;
        setRetryIn(0);
        connect();
        return;
      }
      setRetryIn(left);
      retryTimer = setTimeout(tick, 1000);
    };
    retryTimer = setTimeout(tick, 1000);
  }

  function onClipboard(event) {
    // Text copied inside the desktop goes to the person's clipboard too and into
    // the sidebar's box. A view-only viewer only watches, so nothing is taken out.
    const text = event.detail?.text || '';
    setClipboard(text);
    if (options().viewOnly) return;
    const area = document.createElement('textarea');
    area.value = text;
    area.style.cssText = 'position:fixed;opacity:0';
    document.body.appendChild(area);
    area.select();
    try { document.execCommand('copy'); } catch { /* the desktop shows its own selection */ }
    document.body.removeChild(area);
  }

  function connect() {
    disconnect();
    manualClose = false;
    if (!running()) return;
    setNote(null);
    setReason(null);
    setState('connecting');
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
    // `?view=1` is what the server reads to drop this viewer's input.
    const view = options().viewOnly ? '?view=1' : '';
    // A WebSocket carries no header, so the chosen data center travels in the URL (U01).
    const client = new RFB(screen, environmentUrl(`${scheme}://${location.host}/api/v1/sandboxes/${props.sandbox.id}/desktop${view}`), {
      wsProtocols: ['binary'],
      shared: options().shared,
    });
    rfb = client;
    setClient(client);
    // Local effect at once; the server's answer is the one that counts.
    applyOptions(client, options());
    // An old connection's late events must not speak for the one that replaced
    // it: every handler checks that it still belongs to the current client.
    const current = () => client === rfb;
    client.addEventListener('connect', () => {
      if (!current()) return;
      setState('open');
      setReason(null);
      readSize();
      sizeObserver = new MutationObserver(readSize);
      const canvas = screen?.querySelector('canvas');
      if (canvas) sizeObserver.observe(canvas, { attributes: true, attributeFilter: ['width', 'height'] });
    });
    client.addEventListener('disconnect', (event) => {
      if (!current()) return;
      sizeObserver?.disconnect();
      sizeObserver = null;
      rfb = null;
      setClient(null);
      setSize(null);
      setState('closed');
      // A close this window asked for is the end; anything else is a broken
      // picture, even when the WebSocket itself closed politely.
      if (event.detail.clean && manualClose) {
        setReason('closed from this window');
        return;
      }
      setReason(event.detail.clean ? 'the desktop closed the stream' : 'the desktop did not answer');
      if (!event.detail.clean) {
        setNote('The desktop did not answer. This sandbox probably has no desktop: build it from the "Desktop (XFCE, Chrome, VNC)" pack in the image builder.');
      }
      scheduleReconnect();
    });
    client.addEventListener('clipboard', (event) => {
      if (current()) onClipboard(event);
    });
  }

  function disconnect() {
    cancelReconnect();
    sizeObserver?.disconnect();
    sizeObserver = null;
    manualClose = true;
    const closing = rfb;
    // Cleared first: the close event arrives later and must not be mistaken for
    // a broken picture by the connection that is about to open.
    rfb = null;
    setClient(null);
    try { closing?.disconnect(); } catch { /* already closed */ }
  }

  /** Change some options; scaling and picture quality apply at once. */
  function setOptions(patch) {
    const before = options();
    ui.console.setVncOptions(props.sandbox.id, patch);
    // Shared mode is a handshake and view only is the server's decision: both
    // need the stream opened again.
    const reopens = ('shared' in patch && patch.shared !== before.shared) || ('viewOnly' in patch && patch.viewOnly !== before.viewOnly);
    if (reopens && running()) connect();
  }

  /** Send one named combo (`KEYS`), preceded by any held modifiers. */
  function sendKey(name, sticky = []) {
    if (rfb) sendCombo(rfb, withSticky(sticky, name));
  }

  /** Put text on the desktop's clipboard. */
  function clipboardSend(text) {
    if (!rfb || !text) return;
    rfb.clipboardPasteFrom(text);
    setClipboard(text);
  }

  /**
   * Stop, restart or reset the sandbox from the Power section.
   *
   * A desktop pack answers noVNC's machine power requests, so these go through the
   * sandbox's own lifecycle instead: the same calls the action bar makes.
   */
  async function power(action) {
    if (busy()) return;
    setBusy(true);
    const name = props.sandbox.name || props.sandbox.id;
    try {
      if (action === 'reset') {
        await deck.recreate(props.sandbox.id);
        toast('ok', `${name}: reset; a fresh container is on its way.`);
      } else {
        await sandboxes.act(props.sandbox.id, action);
        toast('ok', `${name}: ${action === 'stop' ? 'stopped' : 'restarted'}.`);
        if (action === 'stop') {
          disconnect();
          setState('closed');
          setReason('the sandbox was stopped');
        }
      }
      props.onChanged?.();
    } catch (failure) {
      toast('bad', failure?.message || `The ${action} failed.`);
    } finally {
      setBusy(false);
    }
  }

  function fullscreen() {
    screen?.requestFullscreen?.();
  }

  async function giveBrowser() {
    try {
      await deck.mcpAttach(props.sandbox.id, 'chrome-browser');
      await deck.reconfigureAgent(props.sandbox.id);
      setNote('The browser is attached. Agents started from now on know it; restart a running agent so it reads its config again.');
      loadBrowser();
    } catch (failure) {
      setNote(failure.message);
    }
  }

  // Options and crop in one effect: they own the same noVNC viewport, so two
  // effects would fight over the order.
  //
  // A monitor tab is always fitted and never asks the desktop to change its
  // resolution — a remote resize would change the framebuffer the crop is a
  // window onto. Everything else uses the person's own scaling option.
  createEffect(() => {
    const live = client();
    // Read the layout revision so an apply (ours or an agent's) re-crops at once.
    display.revision();
    const monitor = activeMonitor();
    const wanted = options();
    if (!live) return;
    if (monitor) {
      applyOptions(live, { ...wanted, scaling: 'none' });
      showMonitor(live, monitor, screenBox());
    } else {
      applyOptions(live, wanted);
      if (wanted.scaling !== 'none') showAll(live);
    }
    screen?.classList.toggle('cropped', Boolean(monitor));
    watchCrop(Boolean(monitor));
  });

  onMount(() => {
    connect();
    loadBrowser();
    props.onReady?.({
      sendCombo: (keys) => (rfb ? sendCombo(rfb, keys) : false),
      sendKey,
      setOptions,
      options,
      clipboardSend,
      lastClipboard: clipboard,
      fullscreen,
      power,
      connect,
      disconnect,
    });
  });
  onCleanup(() => {
    disconnect();
    cancelReconnect();
    if (cropFrame) cancelAnimationFrame(cropFrame);
    cropObserver?.disconnect();
    cropObserver = null;
  });

  // One tab per monitor, remembered per sandbox; a pop-out pins one instead.
  function chooseTab(id) {
    if (props.fixedMonitor) return;
    ui.console.setMonitor(props.sandbox.id, id);
  }

  function popOut(id) {
    window.open(
      environmentUrl(desktopPath(props.sandbox.id, id)),
      `nb-desk-${props.sandbox.id}-${id}`,
      'popup,width=1440,height=900',
    );
  }

  return (
    <section class={`tab-panel ${props.fullWindow ? 'desktop-fill' : ''}`}>
      <Show when={!running()}><p class="muted">The sandbox is not running.</p></Show>
      <Show when={note()}><p class="notice">{note()}</p></Show>
      <Show when={monitors().length > 0 && !props.fixedMonitor}>
        <MonitorTabs
          monitors={monitors()}
          active={activeId()}
          presets={PRESETS}
          onSelect={chooseTab}
          onAdd={(spec) => display.add(spec)}
          onRename={(id, name) => display.rename(id, name)}
          onResize={(id, width, height) => display.resize(id, width, height)}
          onPrimary={(id) => display.primary(id)}
          onPopOut={popOut}
          onRemove={(id) => {
            display.remove(id);
            if (activeId() === id) chooseTab('all');
          }}
        />
      </Show>
      <Show when={props.fixedMonitor && activeMonitor()}>
        {(monitor) => (
          <p class="muted small monitor-pinned">
            {monitor().name} · {monitorSize(monitor())} — this window shows only this monitor.
          </p>
        )}
      </Show>
      <div class="controls">
        <Show when={browser() === false}>
          <button class="tiny" onClick={giveBrowser} title="Attach the Chrome DevTools MCP server so agents can open pages and take screenshots in this Chrome">Give agents this browser (MCP)</button>
        </Show>
        <Show when={browser() === true}>
          <span class="muted small">Agents drive this Chrome through the “chrome-browser” MCP server.</span>
        </Show>
      </div>
      <div class="vnc-wrap">
        <VncSidebar
          sandbox={props.sandbox}
          options={options()}
          onChange={setOptions}
          state={state()}
          connected={state() === 'open'}
          running={running()}
          busy={busy()}
          clipboard={clipboard()}
          display={display}
          monitors={monitors()}
          activeMonitor={activeId()}
          onSelectMonitor={chooseTab}
          onSendClipboard={clipboardSend}
          onSendKey={sendKey}
          onFullscreen={fullscreen}
          onPower={power}
          onReconnect={connect}
          onDisconnect={disconnect}
        />
        <div class="vnc-main">
          <div class={`desktop-screen ${props.fullWindow ? 'full' : ''}`} ref={screen} />
          <VncStatus
            state={state()}
            size={shownSize()}
            quality={options().quality}
            reason={reason()}
            retryIn={retryIn()}
            onReconnect={connect}
            onDisconnect={disconnect}
          />
        </div>
      </div>
    </section>
  );
}
