// Persistent terminal tabs. Each id names a tmux session inside this sandbox.
//
// The tab list lives on the **server**, not in this browser. That is the fix for
// the failure this file used to have: `localStorage` remembered the ids but
// nothing told the sandbox which shells were alive, so a reload could open a
// fresh shell in place of the application the operator had left running. The
// server also knows whether the image has tmux at all, and the tab strip says so
// instead of quietly starting a shell that cannot survive the next reload.
//
// Three things this file learned from the shared shell:
//  * a tab is one tmux session, so several people may sit in it. The strip shows
//    everybody's tabs, each with who is in it, so a tab is joinable instead of
//    invisible;
//  * closing a tab for yourself only forgets it. The old `exit` typed into the
//    shell killed the session under everybody's feet, and it was the browser
//    closing that did it, not a person deciding to;
//  * ending a shell for everyone is its own explicit action, behind a question.

import { For, Show, createEffect, createSignal, onCleanup, onMount } from 'solid-js';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import '@xterm/xterm/css/xterm.css';

/**
 * Put text on the clipboard, also when the console is served over plain HTTP.
 *
 * `navigator.clipboard` exists only in a secure context, and the appliance
 * console is reached by IP over HTTP, so the fallback is a hidden textarea and
 * `execCommand('copy')`, which works from a user gesture anywhere.
 */
function copyText(text, onFailure) {
  if (!text) return;
  if (window.isSecureContext && navigator.clipboard?.writeText) {
    navigator.clipboard.writeText(text).catch(() => {
      if (!legacyCopy(text)) onFailure?.(text);
    });
    return;
  }
  if (!legacyCopy(text)) onFailure?.(text);
}

function legacyCopy(text) {
  const area = document.createElement('textarea');
  area.value = text;
  area.setAttribute('readonly', '');
  area.style.cssText = 'position:fixed;top:0;left:0;opacity:0;pointer-events:none';
  document.body.appendChild(area);
  const focused = document.activeElement;
  area.select();
  let done = false;
  try {
    done = document.execCommand('copy');
  } catch {
    done = false;
  } finally {
    document.body.removeChild(area);
    focused?.focus?.();
  }
  return done;
}
import * as api from '../../api/sandboxes.js';
import * as presence from '../../api/presence.js';
import { environmentUrl } from '../../api/client.js';
import PresenceDots from '../../components/PresenceDots.jsx';
import ConfirmDialog from '../../components/ConfirmDialog.jsx';
// The tab strip is the same list in the Workbox and in the Admin Center, so the
// merge and the SSH lines are shared rather than written twice. `sessions.js` is
// pure logic (no JSX, no state), which is the one kind of module a feature may
// take from another: it cannot create a second truth.
import { mergeTabs, sshCommand, puttyHint } from '../workbox/sessions.js';
// The status line's words and the backoff are pure, and tested without a browser.
import {
  MAX_TRIES,
  closeReasonText,
  nextDelay,
  retryText,
  shouldRetry,
  socketReasonText,
  statusText,
  statusTone,
} from './terminalState.js';
// One EventSource per page: the stream is shared with every other workbox panel.
import { onControl } from '../workbox/useLive.js';
import '../../styles/deck.css';
import '../../styles/workbox-sessions.css';

// `crypto.randomUUID` only exists in a secure context. The appliance is also
// served over plain HTTP on a LAN address, where the property is undefined and
// calling it throws during render — which used to blank the whole sandbox view.
// The fallback keeps the same shape (`t-` plus 20 hex-ish characters) because
// the runtime accepts only ASCII alphanumerics, `_` and `-` as a tab id.
const randomToken = () => {
  const webCrypto = globalThis.crypto;
  if (typeof webCrypto?.randomUUID === 'function') return webCrypto.randomUUID();
  if (typeof webCrypto?.getRandomValues === 'function') {
    const bytes = webCrypto.getRandomValues(new Uint8Array(10));
    return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
  }
  return `${Date.now().toString(16)}${Math.random().toString(16).slice(2, 12)}`.slice(0, 20);
};
const newId = () => `t-${randomToken().replaceAll('-', '').slice(0, 20)}`;
const freshTab = (index) => ({ id: newId(), label: `Terminal ${index}`, alive: false });

export default function TerminalTab(props) {
  let host;
  let socket;
  let term;
  let fit;
  let observer;
  // The tab a pop-out window shows, and nothing else.
  const fixed = () => props.fixedTab || null;
  // On by default, so the Workbox and the Admin Center show the same list and a
  // tab somebody else opened is joinable in both. `showAll={false}` is the plain
  // own-tabs-only view.
  const showAll = () => props.showAll !== false;
  // My own tabs, as the server stores them for me; everybody's, from the presence
  // endpoint. The strip is the merge of the two.
  const [mine, setMine] = createSignal([]);
  const [everyone, setEveryone] = createSignal([]);
  const tabs = () => mergeTabs(mine(), showAll() ? everyone() : []);
  const [active, setActive] = createSignal(fixed());
  // `connecting` | `open` | `closed`, and why it closed when it did.
  const [state, setState] = createSignal('connecting');
  const [reason, setReason] = createSignal(null);
  // Text the browser refused to copy by itself; shown so it can be copied by hand.
  const [copyFallback, setCopyFallback] = createSignal(null);
  const cleanups = [];
  // On by default; remembered per browser.
  const [selectMode, setSelectMode] = createSignal(localStorage.getItem('nestbox.terminal.select') !== 'off');
  const toggleSelect = (on) => { setSelectMode(on); localStorage.setItem('nestbox.terminal.select', on ? 'on' : 'off'); };
  const [note, setNote] = createSignal(null);
  const [durable, setDurable] = createSignal(null);
  const [loading, setLoading] = createSignal(true);
  // The tab whose little menu is open, and the tab whose SSH line is shown.
  const [menu, setMenu] = createSignal(null);
  const [sshFor, setSshFor] = createSignal(null);
  const [confirmTab, setConfirmTab] = createSignal(null);
  // How to reach this sandbox from outside, read the first time somebody asks for
  // the SSH line. The answer costs a runtime query, so it is not asked for before.
  const [access, setAccess] = createSignal(null);
  // Where the right-click menu was opened, and whether a mouse button is down (a
  // drag in progress must not copy on every pixel).
  const [menuAt, setMenuAt] = createSignal(null);

  // ---- the connection ------------------------------------------------------
  //
  // A dropped socket is not the end of the tab: the tmux session is still on the
  // other side, so the console rejoins it by itself, waiting 1 s, 2 s, 5 s and then
  // 10 s between tries, ten tries in all (Proxmox's habit). Everything is cancelled
  // on unmount, because a page you left must not keep reconnecting into a disposed
  // terminal. `Reconnect` starts the count over.
  let retryTimer = null;
  let retryAttempt = 0;
  let stopped = false;
  // Bumped whenever the socket is replaced, so a late handler of the old socket
  // cannot change the state of the new one.
  let generation = 0;
  // The text the selection watcher copied last; the same text is not copied twice.
  let lastCopied = '';
  let selectionTimer = null;
  let pressing = false;
  // A connection counts as healthy after a moment: resetting the backoff on the
  // bare `open` event would turn a crash-looping sandbox into a 1 s hammer.
  let healthyTimer = null;

  const clearRetry = () => {
    if (retryTimer) clearTimeout(retryTimer);
    retryTimer = null;
  };

  /**
   * Drop the socket on purpose: its handlers are removed and its generation is
   * spent, so a tab switch cannot look like a dropped connection and start a
   * reconnect of its own.
   */
  const closeSocket = () => {
    generation += 1;
    clearTimeout(healthyTimer);
    const old = socket;
    socket = null;
    if (!old) return;
    old.onopen = null;
    old.onmessage = null;
    old.onclose = null;
    old.onerror = null;
    try {
      old.close();
    } catch {
      /* already gone */
    }
  };

  /**
   * Ask the API what it thinks of this tab, for the failures the browser cannot see.
   *
   * A refused WebSocket handshake reaches JavaScript as code 1006 with no text: the
   * browser hides the HTTP status on purpose. The tab-list call sits behind the same
   * checks (session, permission, data center, sandbox), so its answer is the status
   * the status line shows instead of a bare "closed".
   */
  async function explainFailure() {
    try {
      await api.terminalTabs(props.sandbox.id);
      return 'the server refused the terminal handshake';
    } catch (failure) {
      if (failure?.status) return `the server answered ${failure.status}: ${failure.message}`;
      return 'the console could not reach the API';
    }
  }

  /** Schedule the next automatic try, or stop and say so after the last one. */
  function reconnectLater(tab, why) {
    if (stopped) return;
    const attempt = retryAttempt;
    if (!shouldRetry(attempt)) {
      setState('closed');
      setReason(
        `${typeof why === 'string' && why.trim() ? `${why.trim()} · ` : ''}gave up after ${MAX_TRIES} tries — press Reconnect`,
      );
      return;
    }
    retryAttempt = attempt + 1;
    setState('connecting');
    setReason(retryText(why, attempt));
    retryTimer = setTimeout(() => {
      retryTimer = null;
      attemptConnect(tab);
    }, nextDelay(attempt));
  }

  /** One connection attempt; `connect` is the fresh start that resets the count. */
  function attemptConnect(tab) {
    clearRetry();
    closeSocket();
    // Everything below belongs to this socket only.
    const run = generation;
    setState('connecting');
    setReason(null);
    setNote(null);
    term?.reset();
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
    // A WebSocket carries no header, so the chosen data center travels in the URL (U01).
    const path = `${scheme}://${location.host}/api/v1/sandboxes/${props.sandbox.id}/terminal?tab=${encodeURIComponent(tab)}`;
    const ws = new WebSocket(environmentUrl(path), ['nestbox-terminal']);
    socket = ws;
    let opened = false;
    let serverClose = null;
    ws.onopen = () => {
      if (run !== generation) return;
      opened = true;
      setState('open');
      setReason(null);
      // The backoff starts over once the connection has proven to hold.
      clearTimeout(healthyTimer);
      healthyTimer = setTimeout(() => { retryAttempt = 0; }, 3000);
      fit?.fit();
      ws.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }));
      // The operator opened this tab to type, so the cursor is where they look.
      term.focus();
    };
    ws.onmessage = (event) => {
      if (run !== generation) return;
      const text = typeof event.data === 'string' ? event.data : '';
      if (text.startsWith('{')) {
        try {
          const frame = JSON.parse(text);
          if (frame?.type === 'close') {
            // The server says why it ended the session; the status line and the
            // note both carry that sentence.
            serverClose = frame;
            setReason(closeReasonText(frame));
            setNote(
              frame.message ||
                `Terminal detached (${frame.reason || 'closed'}). Reconnect attaches it again.`,
            );
            return;
          }
        } catch {
          /* output */
        }
      }
      term.write(text);
    };
    ws.onclose = (event) => {
      if (run !== generation) return;
      socket = null;
      clearTimeout(healthyTimer);
      const text = closeReasonText(serverClose) || socketReasonText(event, opened);
      // Two reasons the server sends are final: the person closed the session, or
      // the sandbox stopped. Repeating a handshake that cannot succeed only makes
      // the page busy.
      const final = serverClose && (serverClose.reason === 'user' || serverClose.reason === 'sandbox_stopped');
      // While a reconnect is pending the line stays "Connecting…": showing
      // "Disconnected" for the fraction of a second before the next try makes the one
      // line an operator watches jump for no reason.
      setState(final ? 'closed' : 'connecting');
      setReason(text || 'the connection closed');
      if (final) return;
      if (!text && !opened) {
        // A refused handshake: the browser kept the status to itself, so ask the
        // API, and retry after that answer is known.
        explainFailure().then((detail) => {
          if (run !== generation) return;
          reconnectLater(tab, detail);
        });
        return;
      }
      reconnectLater(tab, text);
    };
    // No `onerror` note: the close event follows every failed handshake and the status
    // line already says the reason, so a second sentence under the toolbar is noise.
  }

  function connect(tab = active()) {
    if (!tab) return;
    retryAttempt = 0;
    attemptConnect(tab);
  }

  /** Persist the set, then render what the server stored. */
  async function persist(next) {
    setMine(next);
    try {
      const saved = await api.saveTerminalTabs(props.sandbox.id, next);
      setDurable(saved.durable ?? durable());
    } catch (failure) {
      setNote(`The tab list could not be saved: ${failure.message}`);
    }
  }

  /**
   * Everybody's tabs of this sandbox, for the strip and the presence chips.
   *
   * A failure keeps my own list working: merging an empty answer is my list, and
   * a sandbox that cannot answer this question should not lose its terminals
   * over it.
   */
  async function refreshAll() {
    if (!showAll() || fixed()) return;
    try {
      const answer = await presence.tabsAll(props.sandbox.id);
      setEveryone(Array.isArray(answer) ? answer : answer?.items || []);
    } catch {
      setEveryone([]);
    }
  }

  /** Read the server's list and open the first tab's shell. */
  async function load() {
    if (fixed()) {
      // A pop-out shows exactly the tab it was opened for, and reads no list.
      setLoading(false);
      connect(fixed());
      return;
    }
    setLoading(true);
    try {
      const answer = await api.terminalTabs(props.sandbox.id);
      setDurable(answer.durable ?? null);
      let items = (answer.items || []).map((item) => ({
        id: item.tab_id,
        label: item.title,
        alive: item.alive,
      }));
      if (!items.length) {
        // A sandbox with no tabs gets one, and it is stored, so the next reload
        // finds the same shell instead of inventing a second one.
        items = [freshTab(1)];
        await api.saveTerminalTabs(props.sandbox.id, items).catch(() => null);
      }
      setMine(items);
      setActive(items[0].id);
      connect(items[0].id);
    } catch (failure) {
      setNote(`The tab list could not be read: ${failure.message}`);
      const fallback = [freshTab(1)];
      setMine(fallback);
      setActive(fallback[0].id);
      connect(fallback[0].id);
    } finally {
      setLoading(false);
    }
    refreshAll();
  }

  function add() {
    const next = [...mine(), freshTab(mine().length + 1)];
    persist(next);
    setActive(next.at(-1).id);
    queueMicrotask(() => connect(next.at(-1).id));
  }

  function select(id) {
    setMenu(null);
    setSshFor(null);
    if (id === active()) return;
    setActive(id);
    connect(id);
  }

  /** Show whichever tab is left, or nothing when none is. */
  function goTo(remaining, next) {
    const target = remaining[0]?.id ?? next[0]?.id ?? null;
    if (!target) {
      closeSocket();
      setActive(null);
      return;
    }
    closeSocket();
    setActive(target);
    queueMicrotask(() => connect(target));
  }

  /**
   * Remove one tab from my list. The shell keeps running: it may be somebody
   * else's application, and the old `exit` killed it for every viewer.
   */
  function closeMine(id) {
    setMenu(null);
    const remaining = tabs().filter((tab) => tab.id !== id);
    let next = mine().filter((tab) => tab.id !== id);
    if (!next.length && !remaining.length) next = [freshTab(1)];
    const wasActive = id === active();
    persist(next);
    if (wasActive) goTo(remaining, next);
  }

  /**
   * End one tab for everybody: the tmux session stops and every viewer leaves.
   * The only action that destroys work, therefore behind a question.
   */
  async function closeForAll(id) {
    if (!id) return;
    setConfirmTab(null);
    setMenu(null);
    const label = tabs().find((tab) => tab.id === id)?.label || id;
    try {
      await presence.closeTabForAll(props.sandbox.id, id);
      const remaining = tabs().filter((tab) => tab.id !== id);
      let next = mine().filter((tab) => tab.id !== id);
      if (!next.length && !remaining.length) next = [freshTab(1)];
      const wasActive = id === active();
      persist(next);
      if (wasActive) goTo(remaining, next);
      setNote(`“${label}” was closed for everyone.`);
      refreshAll();
    } catch (failure) {
      setNote(`The tab could not be closed for everyone: ${failure.message}`);
    }
  }

  /** Its own window, so a terminal can fill a screen beside another one. */
  function popOut(id) {
    setMenu(null);
    window.open(
      environmentUrl(`/box/${props.sandbox.id}/terminal/${id}`),
      `nb-term-${id}`,
      'popup,width=1200,height=800',
    );
  }

  /** Ask for the address and port the first time, then show the line. */
  async function showSsh(id) {
    if (sshFor() === id) {
      setSshFor(null);
      return;
    }
    setMenu(null);
    setSshFor(id);
    if (access()) return;
    try {
      setAccess(await api.connect(props.sandbox.id));
    } catch {
      setAccess(null);
    }
  }

  // The host is what a client dials: the node's public address, else its
  // hostname. The port is the published SSH port, not the container's 22.
  const sshHost = () => access()?.host || access()?.hostname || access()?.address || null;
  const sshPort = () =>
    (access()?.ports || []).find((port) => port.purpose === 'ssh')?.public_port || null;
  const sshLine = () => sshCommand(sshHost(), sshPort(), sshFor(), access()?.ssh_user);

  // ---- the clipboard -------------------------------------------------------

  /** Copy the selection, if there is one; the same text is not copied twice. */
  function copySelection() {
    const text = term?.hasSelection() ? term.getSelection() : '';
    if (!text) return;
    lastCopied = text;
    copyText(text, setCopyFallback);
  }

  /**
   * The text to paste, from the clipboard where the browser allows reading it.
   *
   * `navigator.clipboard.readText` needs a secure context and a permission the
   * browser may refuse. The console is reached by IP over plain HTTP, where the API
   * does not exist at all, so the fallback is a prompt: it works from a click or a
   * key press, which is exactly when somebody asks to paste.
   */
  async function readClipboard() {
    if (window.isSecureContext && navigator.clipboard?.readText) {
      try {
        return await navigator.clipboard.readText();
      } catch {
        /* refused; ask below */
      }
    }
    return window.prompt('Paste the text to send to the terminal:');
  }

  async function pasteFromClipboard() {
    const text = await readClipboard();
    if (text) term?.paste(text);
    term?.focus();
  }

  /** Focus the terminal after a click into the frame, unless the click was a control. */
  function focusFromClick(event) {
    const tag = event.target?.tagName?.toLowerCase?.() || '';
    if (['button', 'a', 'input', 'textarea', 'select', 'label', 'option'].includes(tag)) return;
    if (event.target?.closest?.('.terminal-menu')) return;
    term?.focus();
  }

  onMount(() => {
    // A terminal that cannot attach must explain itself on its own tab instead
    // of taking the whole sandbox view down with it.
    try {
      term = new Terminal({
        cursorBlink: true,
        fontFamily: 'ui-monospace, Menlo, monospace',
        fontSize: props.fontSize || 13,
        theme: { background: '#000000' },
      });
      fit = new FitAddon();
      term.loadAddon(fit);
      // A URL the program prints (a login link, a pull request) is clickable,
      // also when the terminal wrapped it over several lines.
      term.loadAddon(new WebLinksAddon((_event, uri) => window.open(uri, '_blank', 'noopener,noreferrer')));
      term.open(host);
      fit.fit();
      term.onData((data) => socket?.readyState === WebSocket.OPEN && socket.send(data));
      // xterm keeps its selection internally, so the browser's own copy never
      // sees it. Copy on release of the mouse (a user gesture, which the old
      // clipboard path needs on plain HTTP) and on Ctrl+Shift+C / Ctrl+Insert /
      // Ctrl+C while something is selected. Ctrl+Shift+V pastes from the
      // clipboard, with a prompt when the browser cannot read it (plain HTTP);
      // a right-click opens a small Paste menu for the same thing.
      // xterm finishes the selection in its own document-level mouseup handler,
      // which runs after one on `host`. A zero timeout runs after both, and is
      // still inside the browser's short window in which a click counts as the
      // user's action (the copy command needs that).
      // Full-screen programs (claude, vim, tmux with a mouse) ask the terminal
      // to report the mouse to them, and then a drag selects nothing. xterm
      // lets Shift override that, but nobody guesses it. With "select text" on
      // (the default), a plain click-and-drag is turned into the Shift form
      // before xterm sees it, so selecting and copying always work.
      const onPress = (event) => {
        if (event.button === 0) pressing = true;
        if (!selectMode() || event.shiftKey || event.button !== 0 || event.isTrusted === false) return;
        event.stopImmediatePropagation();
        event.target.dispatchEvent(new MouseEvent('mousedown', {
          bubbles: true, cancelable: true, view: window, shiftKey: true,
          button: 0, buttons: event.buttons, clientX: event.clientX, clientY: event.clientY,
          detail: event.detail, ctrlKey: event.ctrlKey, altKey: event.altKey, metaKey: event.metaKey,
        }));
      };
      host.addEventListener('mousedown', onPress, true);
      cleanups.push(() => host.removeEventListener('mousedown', onPress, true));
      // Anything chosen without the mouse — a keyboard selection, a program
      // selecting text — is copied as well, once it has settled.
      const selectionWatch = term.onSelectionChange(() => {
        if (pressing) return;
        clearTimeout(selectionTimer);
        selectionTimer = setTimeout(() => {
          const text = term?.hasSelection() ? term.getSelection() : '';
          if (!text || text === lastCopied) return;
          copySelection();
        }, 120);
      });
      cleanups.push(() => {
        clearTimeout(selectionTimer);
        selectionWatch?.dispose?.();
      });
      const onRelease = () => {
        pressing = false;
        setTimeout(() => copySelection(), 0);
      };
      document.addEventListener('mouseup', onRelease);
      cleanups.push(() => document.removeEventListener('mouseup', onRelease));
      // The right-click menu: Proxmox's console has one, and on plain HTTP the
      // browser's own Paste is the only other way to get text in. It is not shown
      // when "select text" is off, because then the right button belongs to the
      // program in the terminal (tmux, vim).
      const onContextMenu = (event) => {
        if (!selectMode()) return;
        event.preventDefault();
        setMenuAt({ x: event.clientX, y: event.clientY });
      };
      host.addEventListener('contextmenu', onContextMenu);
      cleanups.push(() => host.removeEventListener('contextmenu', onContextMenu));
      const closeMenu = (event) => {
        if (!menuAt()) return;
        if (event.type === 'keydown' && event.key !== 'Escape') return;
        if (event.type === 'mousedown' && event.target?.closest?.('.terminal-menu')) return;
        setMenuAt(null);
      };
      document.addEventListener('mousedown', closeMenu);
      document.addEventListener('keydown', closeMenu);
      cleanups.push(() => document.removeEventListener('mousedown', closeMenu));
      cleanups.push(() => document.removeEventListener('keydown', closeMenu));
      term.attachCustomKeyEventHandler((event) => {
        if (event.type !== 'keydown') return true;
        const key = event.key.toLowerCase();
        if (event.ctrlKey && event.shiftKey && key === 'v') {
          pasteFromClipboard();
          return false;
        }
        if (!term.hasSelection()) return true;
        const copy =
          (event.ctrlKey && event.shiftKey && key === 'c') ||
          (event.ctrlKey && event.key === 'Insert') ||
          (event.ctrlKey && !event.shiftKey && key === 'c');
        if (!copy) return true;
        copySelection();
        return false;
      });
      observer = new ResizeObserver(() => {
        fit.fit();
        if (socket?.readyState === WebSocket.OPEN) {
          socket.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }));
        }
      });
      observer.observe(host);
      // The console frame drives copy, paste, focus and fit from its toolbar.
      props.onReady?.({
        copy: copySelection,
        paste: (text) => { if (text) term?.paste(text); },
        pasteFromClipboard,
        focus: () => term?.focus(),
        fit: () => fit?.fit(),
        activeTab: () => active(),
      });
    } catch (failure) {
      setState('closed');
      setNote(`The terminal renderer could not start: ${failure?.message || failure}`);
      setLoading(false);
      return;
    }
    // Others join and leave tabs without telling this browser; the strip is
    // refreshed every ten seconds and immediately on a presence event.
    const timer = setInterval(() => refreshAll(), 10000);
    cleanups.push(() => clearInterval(timer));
    load();
  });

  // A font size from the console frame's toolbar applies at once.
  createEffect(() => {
    const size = props.fontSize;
    if (!size || !term) return;
    term.options.fontSize = size;
    fit?.fit();
  });

  // Who is in which tab changes without this tab doing anything, so the strip
  // follows the live stream. A presence event is exactly that change; a restart
  // tells the same story (a session may have been respawned).
  const stopLive = onControl(
    () => props.sandbox.id,
    (event) => {
      if (event.what === 'presence' || event.what === 'restart') refreshAll();
    },
  );

  onCleanup(() => {
    // Nothing may reconnect after the tab is gone: a timer into a disposed
    // terminal throws, and a page you left must not keep asking.
    stopped = true;
    clearRetry();
    clearTimeout(selectionTimer);
    clearTimeout(healthyTimer);
    cleanups.forEach((undo) => undo());
    observer?.disconnect();
    stopLive?.();
    closeSocket();
    term?.dispose();
  });

  return (
    <section class={`tab-panel ${fixed() ? 'sessions-fill' : ''}`} onMouseUp={focusFromClick}>
      <Show when={!fixed()}>
        <div class="controls">
          <Show when={!loading()} fallback={<span class="muted small">Reading the tabs…</span>}>
            <For each={tabs()}>
              {(tab) => (
                <span class={`terminal-tab ${active() === tab.id ? 'active' : ''}`}>
                  <button
                    class="tiny"
                    onClick={() => select(tab.id)}
                    title={tab.created_by_name ? `Opened by ${tab.created_by_name}` : undefined}
                  >
                    {tab.label}
                    <Show when={tab.alive === false}>
                      <span class="muted" title="The shell is not running; connecting starts it again">
                        {' '}· ended
                      </span>
                    </Show>
                  </button>
                  <Show when={tab.people.length > 0}>
                    <PresenceDots people={tab.people} />
                  </Show>
                  <button
                    class="tiny ghost"
                    title="Pop out, SSH, or close this tab"
                    aria-label={`Options for ${tab.label}`}
                    onClick={() => setMenu(menu() === tab.id ? null : tab.id)}
                  >
                    ⋯
                  </button>
                  <Show when={menu() === tab.id}>
                    <span class="terminal-tab-menu">
                      <button class="tiny" onClick={() => popOut(tab.id)}>Pop out</button>
                      <button class="tiny" onClick={() => showSsh(tab.id)}>SSH</button>
                      <button
                        class="tiny"
                        title="Remove the tab from your list; the shell keeps running for everyone else"
                        onClick={() => closeMine(tab.id)}
                      >
                        Close for me
                      </button>
                      <button
                        class="tiny danger"
                        title="End the shell for everybody who is in it"
                        onClick={() => setConfirmTab(tab.id)}
                      >
                        Close for everyone
                      </button>
                    </span>
                  </Show>
                </span>
              )}
            </For>
            <button class="tiny" onClick={add}>+ Terminal</button>
          </Show>
        </div>
      </Show>
      <div class="controls terminal-bar">
        <span
          class={`terminal-status ${statusTone(state())}`}
          role="status"
          aria-live="polite"
          title={reason() || undefined}
        >
          <i />
          {statusText(state(), reason())}
        </span>
        <Show when={state() !== 'open'}>
          <button class="primary tiny" onClick={() => connect()}>Reconnect</button>
        </Show>
        <label class="small" title="Drag with the mouse to select and copy, even inside claude, vim or tmux. Turn off to let those programs use the mouse.">
          <input type="checkbox" checked={selectMode()} onChange={(e) => toggleSelect(e.currentTarget.checked)} /> select text
        </label>
        <span class="muted small">
          Tabs are stored on the server, so a reload or another machine reopens the same shells.
          Selecting copies; Ctrl+Shift+V or a right-click pastes.
        </span>
      </div>
      <Show when={sshFor()}>
        <div class="sessions-ssh">
          <Show
            when={sshLine()}
            fallback={<span class="muted small">Reading how to reach this sandbox…</span>}
          >
            <span class="small">Attach to “{sshFor()}” from your own machine:</span>
            <pre class="mono">{sshLine()}</pre>
            <button class="tiny" onClick={() => copyText(sshLine(), setCopyFallback)}>Copy</button>
            <p class="muted small">{puttyHint(sshHost(), sshPort(), sshFor())}</p>
          </Show>
        </div>
      </Show>
      <Show when={durable() === false}>
        <p class="warn-note">
          This image has no <code>tmux</code>, so a tab cannot survive a reload: reconnecting starts a
          new shell. Recreate the sandbox from an image that includes it to keep applications
          running.
        </p>
      </Show>
      <Show when={note()}>
        <p class="notice">{note()}</p>
      </Show>
      <Show when={copyFallback()}>
        <div class="notice">
          <span class="small">The browser did not allow copying automatically. The text is selected below: press Ctrl+C.</span>
          <textarea
            readonly
            rows="3"
            style="width:100%;font-family:var(--font-mono, monospace)"
            ref={(el) => setTimeout(() => { el.focus(); el.select(); }, 0)}
            onBlur={() => setCopyFallback(null)}
            value={copyFallback()}
          />
        </div>
      </Show>
      <div class="term-wrap">
        <div ref={host} style="height:100%" />
      </div>
      <Show when={menuAt()}>
        <div
          class="terminal-menu"
          role="menu"
          aria-label="Terminal clipboard"
          style={{ left: `${menuAt().x}px`, top: `${menuAt().y}px` }}
        >
          <button type="button" role="menuitem" class="tiny" onClick={() => { setMenuAt(null); pasteFromClipboard(); }}>
            Paste
          </button>
          <button
            type="button"
            role="menuitem"
            class="tiny"
            disabled={!term?.hasSelection()}
            onClick={() => { setMenuAt(null); copySelection(); }}
          >
            Copy
          </button>
        </div>
      </Show>
      <ConfirmDialog
        open={confirmTab() !== null}
        title="Close this tab for everyone?"
        onClose={() => setConfirmTab(null)}
        actions={[
          { label: 'Close for everyone', tone: 'danger', onClick: () => closeForAll(confirmTab()) },
          { label: 'Keep it', tone: 'secondary', onClick: () => setConfirmTab(null) },
        ]}
      >
        <p class="small">
          The shell of “{confirmTab()}” and whatever runs in it end for every person connected to it.
          Closing it only for yourself leaves it running.
        </p>
      </ConfirmDialog>
    </section>
  );
}
