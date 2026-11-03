// The desktop's sidebar, like Proxmox's noVNC panel: clipboard, full screen, settings,
// send key, power, and the status line under the picture.
//
// Props in, callbacks out: the panel never opens a connection itself. Everything it
// shows comes from `options`, `state` and `sandbox`; every click goes back through
// `onChange`, `onSendKey`, `onSendClipboard`, `onFullscreen`, `onReconnect`,
// `onDisconnect` or `onPower`. The caller owns the RFB connection, so the same panel
// serves the console tab and the pop-out window.
//
// `VncStatus` is the second half of the panel: the one line under the screen that says
// what the picture is doing, with the two buttons that fix or end it.
import { For, Show, createEffect, createSignal } from 'solid-js';
import ConfirmDialog from './ConfirmDialog.jsx';
import IconButton from './IconButton.jsx';
import Segmented from './Segmented.jsx';
import Switch from './Switch.jsx';
import { Icon } from './icons.jsx';
import { KEYS, SCALINGS, STICKY, statusText } from '../features/sandboxes/vncOptions.js';
import '../styles/vnc.css';

const SECTIONS = [
  { id: 'clipboard', label: 'Clipboard', icon: 'Copy' },
  { id: 'settings', label: 'Settings', icon: 'Settings' },
  { id: 'keys', label: 'Send key', icon: 'Terminal' },
  { id: 'power', label: 'Power', icon: 'Stop' },
];

const SCALING_LABELS = [
  { value: 'local', label: 'Local scaling', title: 'Scale the picture to this window' },
  { value: 'remote', label: 'Remote resizing', title: 'Ask the desktop to change its resolution to this window' },
  { value: 'none', label: 'None', title: 'One pixel for one pixel, with scrollbars when it does not fit' },
];

const KEYS_GRID = Object.keys(KEYS)
  .filter((name) => /^ctrlAltF\d+$/.test(name))
  .sort((a, b) => Number(a.slice(-1)) - Number(b.slice(-1)));

// The panel on the left edge of the desktop view.
export default function VncSidebar(props) {
  const [collapsed, setCollapsed] = createSignal(false);
  const [section, setSection] = createSignal('clipboard');
  const [text, setText] = createSignal('');
  const [sticky, setSticky] = createSignal([]);
  const [confirm, setConfirm] = createSignal(null);

  // Text copied inside the desktop arrives through the caller; it is the last word,
  // so typing in the box is never overwritten by an older copy.
  createEffect(() => {
    const incoming = props.clipboard;
    if (typeof incoming === 'string') setText(incoming);
  });

  const options = () => props.options || {};
  const change = (patch) => props.onChange?.(patch);
  const sandboxName = () => props.sandbox?.name || 'this sandbox';

  function toggleCollapsed() {
    setCollapsed(!collapsed());
  }

  function openSection(id) {
    setSection(id);
    setCollapsed(false);
  }

  function toggleSticky(name) {
    setSticky((held) => (held.includes(name) ? held.filter((entry) => entry !== name) : [...held, name]));
  }

  // A held modifier is a one-shot: it goes out with the next key and lets go.
  function sendKey(name) {
    props.onSendKey?.(name, sticky());
    setSticky([]);
  }

  const powerTitle = () =>
    confirm() === 'reset'
      ? `Reset ${sandboxName()}?`
      : confirm() === 'restart'
        ? `Restart ${sandboxName()}?`
        : `Stop ${sandboxName()}?`;

  const powerWord = () =>
    confirm() === 'reset' ? `Reset ${sandboxName()}` : confirm() === 'restart' ? `Restart ${sandboxName()}` : `Stop ${sandboxName()}`;

  const powerBody = () =>
    confirm() === 'reset'
      ? 'The container is destroyed and built again from the same image. Only a persistent home survives; anything else written inside is lost.'
      : confirm() === 'restart'
        ? 'The sandbox restarts and its desktop disconnects for a moment. Anything running inside ends.'
        : 'The sandbox stops and its desktop disconnects. Anything running inside ends.';

  return (
    <aside class="vnc-sidebar" classList={{ collapsed: collapsed() }} aria-label="Desktop controls">
      <div class="vnc-side-head">
        <Show when={!collapsed()}>
          <span class="vnc-side-title">Desktop</span>
        </Show>
        <IconButton
          icon={collapsed() ? 'ChevronRight' : 'Close'}
          label={collapsed() ? 'Show the desktop controls' : 'Hide the desktop controls'}
          onClick={toggleCollapsed}
        />
      </div>

      <div class="vnc-nav" role="tablist" aria-label="Desktop sections">
        <For each={SECTIONS}>
          {(entry) => (
            <IconButton
              icon={entry.icon}
              label={entry.label}
              active={section() === entry.id}
              onClick={() => openSection(entry.id)}
            />
          )}
        </For>
      </div>

      <Show when={!collapsed()}>
        <div class="vnc-side-body">
          <button type="button" class="tiny secondary vnc-wide" onClick={() => props.onFullscreen?.()}>
            <Icon name="Desktop" /> Full screen
          </button>

          <Show when={section() === 'clipboard'}>
            <section class="vnc-sec">
              <h4>Clipboard</h4>
              <textarea
                class="vnc-clip"
                aria-label="Clipboard text"
                rows="5"
                placeholder="Type or paste text to send to the desktop"
                value={text()}
                onInput={(event) => setText(event.currentTarget.value)}
              />
              <div class="vnc-row">
                <button type="button" class="tiny" disabled={!text()} onClick={() => props.onSendClipboard?.(text())}>
                  Send
                </button>
                <button type="button" class="tiny secondary" onClick={() => setText('')}>Clear</button>
              </div>
              <p class="vnc-hint">
                Text copied inside the desktop appears here; Send puts this text on the desktop's clipboard.
              </p>
            </section>
          </Show>

          <Show when={section() === 'settings'}>
            <section class="vnc-sec">
              <h4>Scaling</h4>
              <Segmented
                options={SCALING_LABELS}
                value={options().scaling}
                onChange={(value) => change({ scaling: value })}
              />
              <Show when={!SCALINGS.includes(options().scaling)}>
                <p class="vnc-hint">Unknown scaling; the viewer uses local scaling.</p>
              </Show>

              <div class="vnc-toggles">
                <Switch label="View only" checked={options().viewOnly} onChange={(on) => change({ viewOnly: on })} />
                <Switch label="Show dot cursor" checked={options().dotCursor} onChange={(on) => change({ dotCursor: on })} />
                <Switch label="Shared mode" checked={options().shared} onChange={(on) => change({ shared: on })} />
              </div>
              <p class="vnc-hint">Shared mode keeps other viewers connected; it applies on the next connect.</p>

              <label class="vnc-field">
                <span>Quality <b class="mono">{options().quality}</b></span>
                <input
                  type="range"
                  min="0"
                  max="9"
                  step="1"
                  value={options().quality}
                  onInput={(event) => change({ quality: Number(event.currentTarget.value) })}
                />
              </label>
              <label class="vnc-field">
                <span>Compression <b class="mono">{options().compression}</b></span>
                <input
                  type="range"
                  min="0"
                  max="9"
                  step="1"
                  value={options().compression}
                  onInput={(event) => change({ compression: Number(event.currentTarget.value) })}
                />
              </label>

              <div class="vnc-toggles">
                <Switch label="Automatic reconnect" checked={options().reconnect} onChange={(on) => change({ reconnect: on })} />
              </div>
              <label class="vnc-field">
                <span>Reconnect delay <b class="mono">{options().reconnectDelay} s</b></span>
                <input
                  type="range"
                  min="1"
                  max="60"
                  step="1"
                  value={options().reconnectDelay}
                  disabled={!options().reconnect}
                  onInput={(event) => change({ reconnectDelay: Number(event.currentTarget.value) })}
                />
              </label>
            </section>
          </Show>

          <Show when={section() === 'keys'}>
            <section class="vnc-sec">
              <h4>Send key</h4>
              <button type="button" class="tiny vnc-wide" disabled={!props.connected} onClick={() => sendKey('ctrlAltDel')}>
                Ctrl + Alt + Del
              </button>
              <div class="vnc-keys">
                <For each={KEYS_GRID}>
                  {(name) => (
                    <button type="button" class="tiny secondary" disabled={!props.connected} onClick={() => sendKey(name)}>
                      {name.replace('ctrlAlt', 'Ctrl+Alt+')}
                    </button>
                  )}
                </For>
              </div>
              <div class="vnc-row">
                <button type="button" class="tiny secondary" disabled={!props.connected} onClick={() => sendKey('tab')}>Tab</button>
                <button type="button" class="tiny secondary" disabled={!props.connected} onClick={() => sendKey('esc')}>Esc</button>
              </div>
              <h4>Held down</h4>
              <div class="vnc-row vnc-sticky">
                <For each={STICKY}>
                  {(name) => (
                    <button
                      type="button"
                      class="tiny secondary"
                      classList={{ active: sticky().includes(name) }}
                      aria-pressed={sticky().includes(name)}
                      disabled={!props.connected}
                      onClick={() => toggleSticky(name)}
                      title={`Hold ${name} for the next key`}
                    >
                      {name === 'win' ? 'Win' : name === 'ctrl' ? 'Ctrl' : name === 'alt' ? 'Alt' : 'Shift'}
                    </button>
                  )}
                </For>
              </div>
              <p class="vnc-hint">
                A held key is pressed with the next key and released again.
              </p>
            </section>
          </Show>

          <Show when={section() === 'power'}>
            <section class="vnc-sec">
              <h4>Power</h4>
              <p class="vnc-hint">
                A desktop pack answers noVNC's machine power requests, so these use the sandbox's own lifecycle.
              </p>
              <button
                type="button"
                class="tiny danger vnc-wide"
                disabled={props.busy || props.running === false}
                onClick={() => setConfirm('stop')}
              >
                Stop sandbox
              </button>
              <button
                type="button"
                class="tiny secondary vnc-wide"
                disabled={props.busy || props.running === false}
                onClick={() => setConfirm('restart')}
              >
                Restart sandbox
              </button>
              <button
                type="button"
                class="tiny danger vnc-wide"
                disabled={props.busy}
                onClick={() => setConfirm('reset')}
                title="Destroy the container and start a fresh one from the same image"
              >
                Reset (recreate)
              </button>
            </section>
          </Show>
        </div>
      </Show>

      <ConfirmDialog
        open={confirm() !== null}
        title={powerTitle()}
        actions={[
          { label: 'Cancel', onClick: () => setConfirm(null) },
          {
            label: powerWord(),
            tone: 'danger',
            onClick: () => {
              const what = confirm();
              setConfirm(null);
              props.onPower?.(what);
            },
          },
        ]}
      >
        <p>{powerBody()}</p>
      </ConfirmDialog>
    </aside>
  );
}

// The one line under the picture: what the viewer is doing, and the two buttons.
export function VncStatus(props) {
  const tone = () =>
    props.state === 'open' ? 'ok' : props.state === 'connecting' ? 'warn' : props.state === 'closed' ? 'bad' : 'muted';
  return (
    <div class="vnc-status">
      <span class={`badge ${tone()}`}>
        <i />
        {statusText(props.state, { size: props.size, quality: props.quality, reason: props.reason })}
      </span>
      <Show when={props.retryIn}>
        <span class="muted small">reconnecting in {props.retryIn} s</span>
      </Show>
      <span class="vnc-spacer" />
      <Show when={props.state === 'open' || props.state === 'connecting'}>
        <button type="button" class="tiny secondary" onClick={() => props.onDisconnect?.()}>Disconnect</button>
      </Show>
      <button type="button" class="tiny" disabled={props.state === 'connecting'} onClick={() => props.onReconnect?.()}>
        Reconnect
      </button>
    </div>
  );
}
