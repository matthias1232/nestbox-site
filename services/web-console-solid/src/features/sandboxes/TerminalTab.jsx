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
  const [state, setState] = createSignal('connecting');
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

  const closeSocket = () => {
    try {
      socket?.close();
    } catch {
      /* gone */
    }
    socket = null;
  };

  function connect(tab = active()) {
    if (!tab) return;
    closeSocket();
    setState('connecting');
    setNote(null);
    term?.reset();
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
    // A WebSocket carries no header, so the chosen data center travels in the URL (U01).
    socket = new WebSocket(
      environmentUrl(`${scheme}://${location.host}/api/v1/sandboxes/${props.sandbox.id}/terminal?tab=${encodeURIComponent(tab)}`),
      ['nestbox-terminal'],
    );
    socket.onopen = () => {
      setState('open');
      fit?.fit();
      socket.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }));
      term.focus();
    };
    socket.onmessage = (event) => {
      const text = typeof event.data === 'string' ? event.data : '';
      if (text.startsWith('{')) {
        try {
          const frame = JSON.parse(text);
          if (frame?.type === 'close') {
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
    socket.onclose = () => setState('closed');
    socket.onerror = () =>
      setNote('The terminal could not be opened. The sandbox must be running and include the terminal runtime.');
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
      // Ctrl+C while something is selected. Paste (Ctrl+V, Ctrl+Shift+V,
      // Shift+Insert, middle-click) already arrives through the browser.
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
      const onRelease = () => setTimeout(() => {
        if (term?.hasSelection()) copyText(term.getSelection(), setCopyFallback);
      }, 0);
      document.addEventListener('mouseup', onRelease);
      cleanups.push(() => document.removeEventListener('mouseup', onRelease));
      term.attachCustomKeyEventHandler((event) => {
        if (event.type !== 'keydown' || !term.hasSelection()) return true;
        const key = event.key.toLowerCase();
        const copy =
          (event.ctrlKey && event.shiftKey && key === 'c') ||
          (event.ctrlKey && event.key === 'Insert') ||
          (event.ctrlKey && !event.shiftKey && key === 'c');
        if (!copy) return true;
        copyText(term.getSelection(), setCopyFallback);
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
        copy: () => { if (term?.hasSelection()) copyText(term.getSelection(), setCopyFallback); },
        paste: (text) => { if (text) term?.paste(text); },
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
    cleanups.forEach((undo) => undo());
    observer?.disconnect();
    stopLive?.();
    closeSocket();
    term?.dispose();
  });

  return (
    <section class={`tab-panel ${fixed() ? 'sessions-fill' : ''}`}>
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
      <div class="controls">
        <span class={`badge ${state() === 'open' ? 'ok' : state() === 'connecting' ? 'warn' : 'muted'}`}>
          <i />
          {state()}
        </span>
        <Show when={state() === 'closed'}>
          <button class="primary tiny" onClick={() => connect()}>Reconnect</button>
        </Show>
        <label class="small" title="Drag with the mouse to select and copy, even inside claude, vim or tmux. Turn off to let those programs use the mouse.">
          <input type="checkbox" checked={selectMode()} onChange={(e) => toggleSelect(e.currentTarget.checked)} /> select text
        </label>
        <span class="muted small">
          Tabs are stored on the server, so a reload or another machine reopens the same shells.
          Leaving the browser detaches; × removes the tab from your list and leaves the shell running.
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
