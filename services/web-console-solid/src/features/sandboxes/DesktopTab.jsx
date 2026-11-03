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
import { Show, createEffect, createSignal, onCleanup, onMount } from 'solid-js';
import RFB from '@novnc/novnc/lib/rfb';
import * as deck from '../../api/deck.js';
import '../../styles/workbox-sessions.css';

export default function DesktopTab(props) {
  let screen;
  let rfb;
  const [state, setState] = createSignal('idle');
  const [note, setNote] = createSignal(null);
  const [viewOnly, setViewOnly] = createSignal(props.viewOnly === true);
  const [browser, setBrowser] = createSignal(null);
  const running = () => props.sandbox.status === 'running';

  async function loadBrowser() {
    try {
      const attached = await deck.mcpAttached(props.sandbox.id);
      setBrowser((attached.items || attached || []).some?.((item) => (item.server_key || item.key) === 'chrome-browser'));
    } catch {
      setBrowser(null);
    }
  }

  function connect() {
    disconnect();
    if (!running()) return;
    setNote(null);
    setState('connecting');
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
    // `?view=1` is what the server reads to drop this viewer's input.
    const view = viewOnly() ? '?view=1' : '';
    rfb = new RFB(screen, `${scheme}://${location.host}/api/v1/sandboxes/${props.sandbox.id}/desktop${view}`, {
      wsProtocols: ['binary'],
    });
    rfb.scaleViewport = true;
    rfb.resizeSession = false;
    // Immediate local effect while the reconnect is on its way; the server's
    // answer is the one that counts.
    rfb.viewOnly = viewOnly();
    rfb.addEventListener('connect', () => setState('open'));
    rfb.addEventListener('disconnect', (event) => {
      setState('closed');
      if (!event.detail.clean) {
        setNote('The desktop did not answer. This sandbox probably has no desktop: build it from the "Desktop (XFCE, Chrome, VNC)" pack in the image builder.');
      }
    });
    rfb.addEventListener('clipboard', (event) => {
      // Text copied inside the desktop goes to the person's clipboard too. A
      // view-only viewer only watches, so nothing is taken out of it.
      if (viewOnly()) return;
      const area = document.createElement('textarea');
      area.value = event.detail.text;
      area.style.cssText = 'position:fixed;opacity:0';
      document.body.appendChild(area);
      area.select();
      try { document.execCommand('copy'); } catch { /* the desktop shows its own selection */ }
      document.body.removeChild(area);
    });
  }

  function disconnect() {
    try { rfb?.disconnect(); } catch { /* already closed */ }
    rfb = null;
  }

  /** Switch between watching and taking over. The stream is opened again. */
  function setWatch(on) {
    setViewOnly(on);
    if (running()) connect();
  }

  /**
   * Open this desktop in a window of its own.
   *
   * Named per sandbox, so a second click focuses the window that already exists
   * instead of opening a third viewer on the same screen.
   */
  function popOut() {
    window.open(
      `/box/${props.sandbox.id}/desktop`,
      `nb-desk-${props.sandbox.id}`,
      'popup,width=1440,height=900',
    );
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

  // The console frame can switch watching on or off; the stream opens again.
  let lastView = viewOnly();
  createEffect(() => {
    const wanted = props.viewOnly === true;
    if (wanted === lastView) return;
    lastView = wanted;
    setWatch(wanted);
  });

  onMount(() => {
    connect();
    loadBrowser();
    props.onReady?.({
      sendCtrlAltDel: () => rfb?.sendCtrlAltDel(),
      fullscreen: () => screen?.requestFullscreen?.(),
    });
  });
  onCleanup(disconnect);

  return (
    <section class={`tab-panel ${props.fullWindow ? 'desktop-fill' : ''}`}>
      <div class="controls">
        <span class={`badge ${state() === 'open' ? 'ok' : state() === 'connecting' ? 'warn' : 'muted'}`}><i />{state()}</span>
        <button class="tiny" onClick={connect} disabled={!running()}>{state() === 'open' ? 'Reconnect' : 'Connect'}</button>
        <label class="small" title="Watching only: the server drops this window's keyboard, pointer and clipboard, it is not just a setting here.">
          <input type="checkbox" checked={viewOnly()} onChange={(e) => setWatch(e.currentTarget.checked)} />
          {' '}watch only
        </label>
        <button class="tiny secondary" onClick={() => screen?.requestFullscreen?.()}>Full screen</button>
        <Show when={!props.fullWindow}>
          <button class="tiny secondary" onClick={popOut} title="Show this desktop in a window of its own">Pop out</button>
        </Show>
        <Show when={browser() === false}>
          <button class="tiny" onClick={giveBrowser} title="Attach the Chrome DevTools MCP server so agents can open pages and take screenshots in this Chrome">Give agents this browser (MCP)</button>
        </Show>
        <Show when={browser() === true}>
          <span class="muted small">Agents drive this Chrome through the “chrome-browser” MCP server.</span>
        </Show>
      </div>
      <Show when={!running()}><p class="muted">The sandbox is not running.</p></Show>
      <Show when={note()}><p class="notice">{note()}</p></Show>
      <div class={`desktop-screen ${props.fullWindow ? 'full' : ''}`} ref={screen} />
    </section>
  );
}
