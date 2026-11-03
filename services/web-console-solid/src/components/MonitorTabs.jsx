// The monitor tabs of a desktop console: `All`, one tab per monitor, and `+`,
// like the terminal tabs next to them (U10b).
//
// Props in, callbacks out. The strip never reads the layout or writes it: it is
// told which monitors exist, which tab is active, and every click goes back
// through `onSelect`, `onAdd`, `onRename`, `onResize`, `onPrimary`, `onPopOut`
// or `onRemove`. That is what lets the console tab and a pop-out window show the
// same strip.
//
// `⋯` on a tab carries the four things an operator does to one monitor; `+`
// offers the same resolutions as the sidebar, so adding a monitor never needs
// typing numbers unless the person wants to.
import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import IconButton from './IconButton.jsx';
import { Icon } from './icons.jsx';
import { monitorSize } from '../features/sandboxes/displayLayout.js';
import '../styles/display.css';

export default function MonitorTabs(props) {
  const [open, setOpen] = createSignal(null); // { kind: 'add' } | { kind: 'monitor', id }
  const [resizing, setResizing] = createSignal(null); // { id, width, height } while editing a resolution
  let strip;

  function close() {
    setOpen(null);
    setResizing(null);
  }

  function toggle(next) {
    const current = open();
    if (current && current.kind === next.kind && current.id === next.id) {
      close();
      return;
    }
    setOpen(next);
    setResizing(null);
  }

  // An open menu closes when the click is elsewhere or Escape is pressed: a
  // popover that stays open over the picture is in the way.
  const onDown = (event) => {
    if (!open()) return;
    if (strip && !strip.contains(event.target)) close();
  };
  const onKey = (event) => {
    if (event.key === 'Escape' && open()) close();
  };
  onMount(() => {
    document.addEventListener('pointerdown', onDown);
    document.addEventListener('keydown', onKey);
  });
  onCleanup(() => {
    document.removeEventListener('pointerdown', onDown);
    document.removeEventListener('keydown', onKey);
  });

  const monitors = () => props.monitors || [];
  const active = () => props.active || 'all';

  return (
    <div class="monitor-tabs" role="tablist" aria-label="Monitors" ref={strip}>
      <button
        type="button"
        role="tab"
        aria-selected={active() === 'all'}
        class={'monitor-tab' + (active() === 'all' ? ' active' : '')}
        onClick={() => { close(); props.onSelect?.('all'); }}
      >
        All
        <span class="mono muted small">{monitors().length > 1 ? `${monitors().length} screens` : ''}</span>
      </button>

      <For each={monitors()}>
        {(monitor) => (
          <div class={'monitor-tab' + (active() === monitor.id ? ' active' : '')}>
            <button
              type="button"
              role="tab"
              aria-selected={active() === monitor.id}
              title={`${monitor.name} · ${monitorSize(monitor)}${monitor.primary ? ' · primary' : ''}`}
              onClick={() => { close(); props.onSelect?.(monitor.id); }}
            >
              {monitor.name}
              <span class="mono muted small">{monitorSize(monitor)}</span>
              <Show when={monitor.primary}><span class="monitor-primary" title="Primary monitor">●</span></Show>
            </button>
            <IconButton
              icon="Settings"
              label={`Options for ${monitor.name}`}
              active={open()?.kind === 'monitor' && open()?.id === monitor.id}
              onClick={() => toggle({ kind: 'monitor', id: monitor.id })}
            />

            <Show when={open()?.kind === 'monitor' && open()?.id === monitor.id}>
              <div class="monitor-menu" role="menu">
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    const name = window.prompt(`Name for ${monitor.name}:`, monitor.name);
                    if (name) props.onRename?.(monitor.id, name);
                    close();
                  }}
                >
                  Rename…
                </button>
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => setResizing({ id: monitor.id, width: monitor.width, height: monitor.height })}
                >
                  Resolution…
                </button>
                <button
                  type="button"
                  role="menuitem"
                  disabled={monitor.primary === true}
                  onClick={() => { props.onPrimary?.(monitor.id); close(); }}
                >
                  Make primary
                </button>
                <button type="button" role="menuitem" onClick={() => { props.onPopOut?.(monitor.id); close(); }}>
                  Pop out
                </button>
                <button
                  type="button"
                  role="menuitem"
                  class="danger"
                  disabled={monitors().length <= 1}
                  onClick={() => { props.onRemove?.(monitor.id); close(); }}
                >
                  Remove
                </button>

                <Show when={resizing()?.id === monitor.id}>
                  <form
                    class="monitor-size"
                    onSubmit={(event) => {
                      event.preventDefault();
                      props.onResize?.(monitor.id, resizing().width, resizing().height);
                      close();
                    }}
                  >
                    <label>
                      W
                      <input
                        type="number"
                        min="320"
                        max="7680"
                        class="mono"
                        value={resizing().width}
                        onInput={(event) => setResizing({ ...resizing(), width: Number(event.currentTarget.value) })}
                      />
                    </label>
                    <label>
                      H
                      <input
                        type="number"
                        min="320"
                        max="7680"
                        class="mono"
                        value={resizing().height}
                        onInput={(event) => setResizing({ ...resizing(), height: Number(event.currentTarget.value) })}
                      />
                    </label>
                    <button type="submit" class="tiny">Apply</button>
                  </form>
                </Show>
              </div>
            </Show>
          </div>
        )}
      </For>

      <div class="monitor-add">
        <IconButton
          icon="Plus"
          label="Add a monitor"
          active={open()?.kind === 'add'}
          onClick={() => toggle({ kind: 'add' })}
        />
        <Show when={open()?.kind === 'add'}>
          <AddMonitor presets={props.presets || []} onCancel={close} onAdd={(spec) => { props.onAdd?.(spec); close(); }} />
        </Show>
      </div>
    </div>
  );
}

// The `+` popover: a preset or a size of one's own, and where it goes.
function AddMonitor(props) {
  const presets = () => props.presets || [];
  const [choice, setChoice] = createSignal(String(presets()[0]?.label || 'custom'));
  const [width, setWidth] = createSignal(presets()[0]?.width || 1920);
  const [height, setHeight] = createSignal(presets()[0]?.height || 1080);
  const [position, setPosition] = createSignal('right');

  const custom = () => choice() === 'custom';

  function pick(label) {
    setChoice(label);
    const found = presets().find((entry) => entry.label === label);
    if (found) {
      setWidth(found.width);
      setHeight(found.height);
    }
  }

  return (
    <div class="monitor-add-form">
      <label class="monitor-field">
        <span>Resolution</span>
        <select value={choice()} onChange={(event) => pick(event.currentTarget.value)}>
          <For each={presets()}>
            {(entry) => <option value={entry.label}>{entry.label}</option>}
          </For>
          <option value="custom">Custom…</option>
        </select>
      </label>
      <Show when={custom()}>
        <div class="monitor-row">
          <label class="monitor-field">
            <span>W</span>
            <input type="number" min="320" max="7680" class="mono" value={width()} onInput={(event) => setWidth(Number(event.currentTarget.value))} />
          </label>
          <label class="monitor-field">
            <span>H</span>
            <input type="number" min="320" max="7680" class="mono" value={height()} onInput={(event) => setHeight(Number(event.currentTarget.value))} />
          </label>
        </div>
      </Show>
      <label class="monitor-field">
        <span>Place</span>
        <select value={position()} onChange={(event) => setPosition(event.currentTarget.value)}>
          <option value="right">Right of the others</option>
          <option value="below">Below the others</option>
        </select>
      </label>
      <div class="monitor-row">
        <button
          type="button"
          class="tiny"
          onClick={() => props.onAdd?.({ width: width(), height: height(), position: position() })}
        >
          <Icon name="Plus" /> Add monitor
        </button>
        <button type="button" class="tiny secondary" onClick={props.onCancel}>Cancel</button>
      </div>
    </div>
  );
}
